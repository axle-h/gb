use std::cmp::PartialEq;
use std::collections::{HashMap, HashSet};
use std::fmt::{Display, Formatter};
use gb::geometry::Point8;
use gb::joypad::JoypadButton;
use crate::pokemon::map::Map;
use crate::pokemon::actions::OverworldAction;
use crate::pokemon::map_metadata::{CurrentMap, PlayerFacingDirection};
use crate::pokemon::tile::JumpDirection;
use crate::pokemon::sprite::Sprite;
use crate::pokemon::tile::{HiddenObject, MetaTile};

#[derive(Debug, Clone, Default)]
pub struct MetaTileMap {
    pub player_position: Point8,
    /// False while a map transition is in flight, and everything that draws a conclusion from
    /// [`Self::player_position`] has to check it.
    pub position_settled: bool,
    /// The player is on the water rather than on foot (`wWalkBikeSurfState == 2`).
    pub surfing: bool,
    pub player_direction: PlayerFacingDirection,
    pub map: Map,
    pub width: usize,
    pub height: usize,
    pub meta_tiles: Vec<MetaTile>,
    /// Bottom-left raw tile ID of each meta-tile (parallel to `meta_tiles`). Used to evaluate
    /// `tile_pair_collisions` during BFS.
    pub raw_tile_ids: Vec<u8>,
    /// This map's tileset. Kept so per-tileset ROM tables can be consulted against `raw_tile_ids`
    /// — currently [`crate::pokemon::map_header::TileSetId::warp_tile_ids`], via
    /// [`Self::is_step_on_warp`].
    pub tileset: crate::pokemon::map_header::TileSetId,
    /// Unordered raw-tile-ID pairs the player may not walk between in this tileset (elevation
    /// boundaries from pokered `TilePairCollisionsLand`). Empty for most tilesets.
    pub tile_pair_collisions: Vec<(u8, u8)>,
    /// [`Self::walkable_bits`]'s answer, computed at most once per instance.
    walkable_cache: std::cell::OnceCell<Vec<u64>>,
    /// The pairs that apply when water is on either side of the move — mounting Surf, stepping
    /// ashore, or moving while surfing (pokered `TilePairCollisionsWater`). In the Cavern tileset
    /// this is `($14, $05)`: inside Seafoam the player can only get on/off the water at a shore
    /// tile, never straight off a plain cave floor.
    pub tile_pair_collisions_water: Vec<(u8, u8)>,
    pub sprites: Vec<Sprite>,
    /// Each person's square and the tile they are standing on top of — see
    /// [`CurrentMap::underfoot`](crate::pokemon::map_metadata::CurrentMap::underfoot). Read only
    /// by [`Self::row_blocked_by_people`].
    pub underfoot: Vec<(Point8, MetaTile)>,
    /// Unique `(destination_map, destination_position)` pairs reachable via warp tiles. Keyed on
    /// destination position so that two staircase/door warps that lead to *different* positions
    /// within the same destination map (e.g. Mt Moon B1F) each produce a separate
    /// `OverworldAction`.
    pub warp_targets: HashSet<(Map, Point8)>,
    pub connection_targets: HashSet<Map>,
    /// Arrow (spinner) tiles → the tile the forced slide deposits the player on. Stepping onto an
    /// arrow tile hands control to the game, which slides the player along a fixed path (decoded
    /// from the ROM `RocketHideout{2,3}ArrowTilePlayerMovement` tables).
    pub spinners: HashMap<Point8, Point8>,
    /// `wMovementFlags`' `BIT_STANDING_ON_WARP`: whether the player's last completed step landed
    /// on a warp entry.
    pub standing_on_warp: bool,
    /// Squares on this map whose warp the map's own script cancels, in this map's padded
    /// coordinates. See [`map_warp_gate_specs`](crate::pokemon::map_metadata) for the argument;
    /// [`Self::warp_trigger`] answers `Impossible` for them, which is what keeps the row out of
    /// [`Self::actions`] and out of `OverworldMovement`'s border-warp arm at the same time.
    pub script_cancelled_warps: Vec<Point8>,
    /// True when the player can Surf here: Soul Badge, a party mon that knows Surf, and not being
    /// force-ridden on the bike (`IsSurfingAllowed` refuses Surf on Cycling Road, and Routes
    /// 16–18 run along the sea, so believing otherwise routes the BFS straight down the water).
    /// When set, the BFS treats `Water` tiles as passable so routes cross water; the agent mounts
    /// Surf at the land↔water boundary.
    pub can_surf: bool,
    /// The best fishing rod in the bag, or `None` when there is not one. Set by `game_state()`
    /// after construction for the same reason [`Self::can_surf`] is — the map builder has no bag
    /// access — and read by [`Self::actions`], which offers a `MetaTile::Fish` row only when it
    /// is `Some` and this map has water the player can face.
    pub best_rod: Option<crate::pokemon::postgame::fishing::Rod>,
    /// True when the player can Cut here: the Cascade Badge and a party mon that knows Cut
    /// (`GameState::can_use_cut`). Set by `game_state()` after construction, for the same reason
    /// [`Self::can_surf`] is — the map builder has no party access.
    pub can_cut: bool,
    /// True when the player can use Strength here: the Rainbow Badge and a party mon that knows
    /// it. Set by `game_state()` after construction, for the same reason [`Self::can_cut`] is.
    pub can_strength: bool,
    /// Is Bill waiting inside his own machine?
    pub bill_cell_separator: bool,
    /// Strength boulder-switch tiles on this map (invisible pressure plates, from the ROM map
    /// scripts): push a boulder onto one to open its barrier. Exposed so a policy (deterministic
    /// or LLM) can discover *where* to push without hardcoding coordinates.
    pub strength_switches: Vec<Point8>,
    /// Floor-hole tiles on this map (Victory Road 3F): the player can fall through one to the
    /// floor below, and pushing a boulder onto one drops it there (revealing a hidden boulder).
    /// Also modelled as `MetaTile::Warp` for routing (see `apply_victory_road_holes`); this list
    /// is for discovery.
    pub holes: Vec<Point8>,
    /// Land tiles the player may not mount Surf from (Seafoam B4F's (7,11) — "The current is much
    /// too fast!"). One-way: stepping ashore onto them is still allowed.
    pub no_surf_mount: HashSet<Point8>,
    // There was a `hidden_items` here, decoded from the ROM's own two tables and corrected for
    // the connection strip.
    pub has_grass_encounters: bool,
    /// The metadata these tiles were classified from, kept so anything that wants the map's
    /// *pixels* — [`crate::pokemon::map_gfx`] and the picture the model is sent — can reach the
    /// block map, the blockset and the connection strips without re-reading the MMU.
    pub metadata: Option<std::sync::Arc<crate::pokemon::map_metadata::MapMetadata>>,
}

/// Strength boulder-switch tiles per map (raw object/script coords, no connection offset), from
/// the pokered map scripts (e.g. `VictoryRoad1F.asm` `.SwitchCoords`).
fn strength_switch_table(map: Map) -> &'static [(u8, u8)] {
    match map {
        Map::VictoryRoad1F => &[(17, 13)],
        Map::VictoryRoad2F => &[(1, 16), (9, 16)],
        Map::VictoryRoad3F => &[(3, 5)],
        _ => &[],
    }
}

/// Floor-hole tiles per map (raw coords): a boulder pushed onto one falls to the floor below —
/// and so does the player.
fn hole_table(map: Map) -> &'static [(u8, u8)] {
    match map {
        Map::VictoryRoad3F => &[(23, 15)],
        // Pokered `Seafoam{1,2,3,4}HolesCoords`.
        Map::SeafoamIslands1F  => &[(17, 6), (24, 6)],
        Map::SeafoamIslandsB1F => &[(18, 6), (23, 6)],
        Map::SeafoamIslandsB2F => &[(19, 6), (22, 6)],
        Map::SeafoamIslandsB3F => &[(3, 16), (6, 16)],
        _ => &[],
    }
}

/// Land tiles from which the game refuses to let the player *mount* Surf, even though they sit
/// next to water (raw coords).
fn no_surf_mount_table(map: Map) -> &'static [(u8, u8)] {
    match map {
        Map::SeafoamIslandsB4F => &[(7, 11)],
        _ => &[],
    }
}

/// Arrow-tile → slide-destination tables for the spinner-floor maps (raw map coords), decoded
/// from the ROM movement RLE tables (`RocketHideout{2,3}ArrowTilePlayerMovement`, read backwards;
/// PAD_DOWN=+y, UP=−y, LEFT=−x, RIGHT=+x).
fn spinner_table(map: Map) -> &'static [(u8, u8, u8, u8)] {
    match map {
        Map::RocketHideoutB2F => &[
            (4,9,2,9),(4,11,8,11),(4,15,8,11),(4,16,8,11),(4,19,2,19),(4,22,2,19),(5,14,9,16),
            (6,22,6,20),(6,24,6,20),(8,9,2,9),(8,12,8,11),(8,15,8,11),(8,19,2,19),(8,23,2,19),
            (9,14,9,16),(9,22,9,24),(10,9,2,9),(10,10,2,9),(10,15,2,9),(10,17,14,15),(10,19,14,15),
            (10,25,14,25),(11,14,15,18),(11,16,15,18),(11,18,11,20),(12,9,2,9),(12,11,2,9),(12,13,2,9),
            (12,17,14,15),(13,10,14,12),(13,12,14,12),(13,16,15,18),(13,18,11,20),(13,19,14,15),
            (13,22,9,24),(13,23,2,19),(14,17,14,15),(15,16,15,18),(16,14,16,13),(16,16,16,13),
            (16,18,16,13),(17,10,14,12),(17,11,2,9),
        ],
        Map::RocketHideoutB3F => &[
            (10,13,14,13),(10,19,18,15),(11,18,15,22),(12,11,10,11),(12,17,18,15),(12,20,18,15),
            (13,16,17,16),(14,11,16,11),(14,15,18,15),(14,17,18,15),(14,19,18,15),(15,16,17,16),
            (15,18,15,22),(16,13,16,11),(17,12,17,16),(18,16,18,15),
        ],
        // Viridian Gym (Giovanni / Earth Badge), decoded from
        // `ViridianGymArrowTilePlayerMovement` (all single-segment `db PAD_DIR, N`): each arrow
        // at (x,y) slides N tiles → (tx,ty).
        Map::ViridianGym => &[
            (19,11,19,2),  // UP 9
            (19,1,11,1),   // LEFT 8
            (18,2,18,11),  // DOWN 9
            (11,2,17,2),   // RIGHT 6
            (16,10,16,12), // DOWN 2
            (4,6,4,13),    // DOWN 7
            (5,13,13,13),  // RIGHT 8
            (4,14,13,14),  // RIGHT 9
            (0,15,0,7),    // UP 8
            (1,15,1,9),    // UP 6
            (13,16,7,16),  // LEFT 6
            (13,17,1,17),  // LEFT 12
        ],
        _ => &[],
    }
}

/// How the cartridge can be made to take a warp entry the player is standing on. See
/// [`MetaTileMap::warp_trigger`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarpTrigger {
    /// The tile is a door or a warp tile in its own right: arriving on it warps, no button
    /// needed.
    StepOn,
    /// It warps only while this direction is held, either as the last step of the walk onto it or
    /// as a bump into the wall from on top of it.
    HoldDirection(JoypadButton),
    /// Nothing triggers it from this side.
    Impossible,
    /// The check depends on a tile this model does not hold, so nothing is claimed either way.
    Unknown,
}

/// One way off this map into an adjacent one: a run of touching edge tiles, not a tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Crossing {
    /// The tile to leave by, in this map's action-id coordinates. The reachable member of the run
    /// nearest the player when there is one, so the id names a square that can actually be walked
    /// to.
    pub at: Point8,
    /// Where it lands on the far map, in that map's raw coordinates. Paired with `at` because
    /// [`MetaTileMap::connection_action`] is keyed on it, and two runs into the same map differ
    /// only here.
    pub to_position: Point8,
    /// Whether any tile of the run can be walked to from where the player is standing.
    pub reachable: bool,
    /// How many tiles the run holds.
    pub tiles: usize,
}

/// What the search charges for a step from land onto water, in walking steps.
const SURF_MOUNT_COST: u32 = 10;

/// The answer to one `solve_boulder_push_tracking` question, and everything that answer depends
/// on.
#[derive(PartialEq, Eq, Hash)]
struct PlanKey {
    /// Covers `raw_tile_ids` and `tile_pair_collisions`, which `pair_blocked` and
    /// `boulder_push_terrain_refusal` read and which are fixed for a given map.
    map: Map,
    /// Two bits per tile, in reading order: may the player stand here, may a boulder land here.
    walkable: Vec<u64>,
    /// The visible boulders, in the search's own canonical order (`tracked` first when there is
    /// one).
    layout: Vec<Point8>,
    /// The player's *component*, not their square — the lowest tile they can reach with this
    /// layout as walls.
    component: Point8,
    tracked: Option<Point8>,
    target: Point8,
}

thread_local! {
    static PLAN_CACHE: std::cell::RefCell<std::collections::HashMap<PlanKey, Option<Vec<(Point8, JoypadButton)>>>>
        = std::cell::RefCell::new(std::collections::HashMap::new());
}

/// Entries kept before the cache is emptied.
const PLAN_CACHE_CAP: usize = 4_096;

/// Every `(map, landing)` a warp tile in `tiles` leads to.
fn warp_targets_of(tiles: &[MetaTile]) -> HashSet<(Map, Point8)> {
    tiles.iter()
        .filter_map(|t| if let MetaTile::Warp { to_map, to_position } = t { Some((*to_map, *to_position)) } else { None })
        .collect()
}

/// Every adjacent map `tiles` touches, by land or by water.
fn connection_targets_of(tiles: &[MetaTile]) -> HashSet<Map> {
    tiles.iter()
        .filter_map(|t| match t {
            MetaTile::Connection { to_map, .. } => Some(*to_map),
            MetaTile::ConnectionWater(to_map) => Some(*to_map),
            _ => None,
        })
        .collect()
}

impl MetaTileMap {
    pub fn new(map: &CurrentMap) -> Self {
        let dimensions = map.metadata.dimensions();
        let width  = dimensions.full_width();
        let height = dimensions.full_height();
        // Clamp to valid tile coordinates.
        let unclamped_x = map.player_position.x as usize + dimensions.west_extra;
        let unclamped_y = map.player_position.y as usize + dimensions.north_extra;
        let px = unclamped_x.min(width.saturating_sub(1)) as u8;
        let py = unclamped_y.min(height.saturating_sub(1)) as u8;
        let meta_tiles = map.meta_tiles();
        Self {
            walkable_cache: std::cell::OnceCell::new(),
            position_settled: map.header_loaded && map.sprites_loaded
                && unclamped_x < width && unclamped_y < height,
            surfing: map.surfing,
            player_position: Point8 { x: px, y: py },
            player_direction: map.player_direction,
            map: map.metadata.map,
            width,
            height,
            sprites: map.sprites.iter().map(|s| {
                let mut s = *s;
                s.position.x += dimensions.west_extra as u8;
                s.position.y += dimensions.north_extra as u8;
                s
            }).collect(),
            underfoot: map.underfoot(),
            warp_targets: warp_targets_of(&meta_tiles),
            connection_targets: connection_targets_of(&meta_tiles),
            has_grass_encounters: map.grass_encounter_rate != 0,
            raw_tile_ids: map.metadata.raw_tile_ids.clone(),
            tileset: map.metadata.map_header.tileset,
            tile_pair_collisions: map.metadata.tile_pair_collisions.clone(),
            tile_pair_collisions_water: map.metadata.tile_pair_collisions_water.clone(),
            spinners: spinner_table(map.metadata.map).iter().map(|&(x, y, tx, ty)| {
                let off = |px: u8, py: u8| Point8 {
                    x: px + dimensions.west_extra as u8,
                    y: py + dimensions.north_extra as u8,
                };
                (off(x, y), off(tx, ty))
            }).collect(),
            standing_on_warp: map.standing_on_warp,
            script_cancelled_warps: map.script_cancelled_warps.iter().map(|p| Point8 {
                x: p.x + dimensions.west_extra as u8,
                y: p.y + dimensions.north_extra as u8,
            }).collect(),
            meta_tiles,
            can_surf: false,
            best_rod: None,
            can_cut: false,
            can_strength: false,
            bill_cell_separator: false,
            strength_switches: strength_switch_table(map.metadata.map).iter()
                .map(|&(x, y)| Point8 { x: x + dimensions.west_extra as u8, y: y + dimensions.north_extra as u8 })
                .collect(),
            holes: hole_table(map.metadata.map).iter()
                .map(|&(x, y)| Point8 { x: x + dimensions.west_extra as u8, y: y + dimensions.north_extra as u8 })
                .collect(),
            no_surf_mount: no_surf_mount_table(map.metadata.map).iter()
                .map(|&(x, y)| Point8 { x: x + dimensions.west_extra as u8, y: y + dimensions.north_extra as u8 })
                .collect(),
            // Already `Arc`'d and cached in `MapMetadataCache`, so this is a refcount bump.
            metadata: Some(std::sync::Arc::clone(&map.metadata)),
        }
    }

    /// A direction to an `Empty` (freely walkable, not pair-blocked) neighbour of `pos`, if any.
    fn walkable_neighbor_dir(&self, pos: Point8) -> Option<JoypadButton> {
        let neighbors = [
            (JoypadButton::Down,  Point8 { x: pos.x,                 y: pos.y.wrapping_add(1) }),
            (JoypadButton::Up,    Point8 { x: pos.x,                 y: pos.y.wrapping_sub(1) }),
            (JoypadButton::Left,  Point8 { x: pos.x.wrapping_sub(1), y: pos.y                 }),
            (JoypadButton::Right, Point8 { x: pos.x.wrapping_add(1), y: pos.y                 }),
        ];
        neighbors.into_iter().find_map(|(dir, nb)| {
            let inb = (nb.x as usize) < self.width && (nb.y as usize) < self.height;
            (inb
                && !self.pair_blocked(pos, nb)
                && self.meta_tiles[nb.x as usize + nb.y as usize * self.width] == MetaTile::Empty)
                .then_some(dir)
        })
    }

    /// True if the player may not step between meta-tiles `a` and `b` because their bottom-left
    /// raw tile IDs form a forbidden pair in this tileset. The check is symmetric, matching
    /// `CheckForTilePairCollisions`.
    pub(crate) fn pair_blocked(&self, a: Point8, b: Point8) -> bool {
        let is_water = |p: Point8| matches!(
            self.meta_tiles[p.x as usize + p.y as usize * self.width],
            MetaTile::Water | MetaTile::ConnectionWater(_));
        let table = if is_water(a) || is_water(b) {
            &self.tile_pair_collisions_water
        } else {
            &self.tile_pair_collisions
        };
        if table.is_empty() { return false; }
        let ta = self.raw_tile_ids[a.x as usize + a.y as usize * self.width];
        let tb = self.raw_tile_ids[b.x as usize + b.y as usize * self.width];
        table.iter().any(|&(t1, t2)| {
            (ta == t1 && tb == t2) || (ta == t2 && tb == t1)
        })
    }

    pub fn tile_at(&self, point: Point8) -> MetaTile {
        self.meta_tiles[point.x as usize + point.y as usize * self.width]
    }

    /// True if the warp on `point` is the kind that fires the moment you step onto it
    /// (`CheckWarpsNoCollision`), rather than the map-edge kind that needs the outward direction
    /// pressed. See [`crate::pokemon::map_header::TileSetId::warp_tile_ids`] for why the two
    /// cannot be told apart by position.
    pub fn is_step_on_warp(&self, point: Point8) -> bool {
        let index = point.x as usize + point.y as usize * self.width;
        self.raw_tile_ids.get(index)
            .is_some_and(|id| self.tileset.warp_tile_ids().contains(id))
    }

    /// Bounds-checked `tile_at` — `None` if `point` is off the map.
    pub fn tile_at_checked(&self, point: Point8) -> Option<MetaTile> {
        if (point.x as usize) < self.width && (point.y as usize) < self.height {
            Some(self.meta_tiles[point.x as usize + point.y as usize * self.width])
        } else {
            None
        }
    }

    /// Follow the arrow-tile chain from `pos` to the tile the forced slide finally rests on.
    fn resolve_spinner(&self, pos: Point8) -> Point8 {
        let mut cur = pos;
        for _ in 0..64 {
            match self.spinners.get(&cur) {
                Some(&next) => cur = next,
                None => break,
            }
        }
        cur
    }

    pub fn player_tile(&self) -> MetaTile {
        self.tile_at(self.player_position)
    }

    /// Returns every warp and connection tile that is reachable by BFS from the player position,
    /// together with its expanded-coordinate position in the map.
    pub fn all_reachable_warps_and_connections(&self) -> Vec<(Point8, MetaTile)> {
        let (dist, _) = self.bfs_from_player();
        self.meta_tiles
            .iter()
            .enumerate()
            .filter_map(|(i, t)| {
                if !matches!(t, MetaTile::Warp { .. } | MetaTile::Connection { .. } | MetaTile::ConnectionWater(_)) {
                    return None;
                }
                let pos = Point8 { x: (i % self.width) as u8, y: (i / self.width) as u8 };
                dist.contains_key(&pos).then_some((pos, *t))
            })
            .collect()
    }

    /// The set of tiles reachable from the player (debug/diagnostic aid for maze mapping). Every
    /// tile the player can route to from where they are standing.
    pub fn reachable_tiles(&self) -> std::collections::HashSet<Point8> {
        self.bfs_from_player().0.into_keys().collect()
    }

    /// A wander action to the farthest reachable WALKABLE tile: walking (or Surfing) there
    /// triggers a per-step encounter on a cave/water map that has no grass and no reachable cave
    /// object to pace toward (e.g. entering Seafoam in a pocket away from the boulders). Only
    /// Empty/Grass/Water destinations are considered — never a Warp/Connection tile (stepping
    /// onto one would leave the map).
    pub fn wander_action(&self) -> Option<crate::pokemon::actions::OverworldAction> {
        // Steps, not the price `bfs_from_player` reports.
        let (_, steps, _) = self.search_from_player();
        let dest = steps.iter()
            .filter(|(p, _)| matches!(
                self.meta_tiles[p.x as usize + p.y as usize * self.width],
                MetaTile::Empty | MetaTile::Grass | MetaTile::Water))
            .max_by_key(|(_, d)| **d)
            .map(|(p, _)| *p)?;
        let route = self.route_to(dest)?;
        (!route.is_empty()).then(|| crate::pokemon::actions::OverworldAction {
            map: self.map, origin: self.player_position, destination: dest,
            tile: self.meta_tiles[dest.x as usize + dest.y as usize * self.width], route,
        })
    }

    /// The raw tile id `CheckForCollisionWhenPushingBoulder` refuses a boulder onto by id, as a
    /// special case sitting beside the tileset's own collision list.
    const BOULDER_STAIRS_TILE: u8 = 0x15;

    /// The two refusals a boulder push gets that ordinary walking does not, given the tile the
    /// player would be standing on and the tile the boulder would be pushed onto: `Some(reason)`
    /// if the cartridge would refuse.
    fn boulder_push_terrain_refusal(&self, stand: Point8, dest: Point8) -> Option<String> {
        // Two rules, two sentences: they are refused at the same moment and are nothing alike,
        // and a model told "a step up or down" about a staircase will look for a cliff that is
        // not there.
        if self.raw_tile_ids[dest.x as usize + dest.y as usize * self.width] == Self::BOULDER_STAIRS_TILE {
            return Some(format!(
                "there are stairs at ({}, {}), and a boulder will not go onto stairs", dest.x, dest.y));
        }
        if self.pair_blocked(stand, dest) {
            return Some(format!(
                "({}, {}) is a step up or down from where you would be standing, and a boulder will \
                 not go over one", dest.x, dest.y));
        }
        None
    }

    /// Every boulder actually standing on this map, in reading order.
    pub fn boulders(&self) -> Vec<Point8> {
        let mut found: Vec<Point8> = self.sprites.iter()
            .filter(|s| s.name.starts_with("Boulder") && !s.hidden)
            .map(|s| s.position).collect();
        found.sort_by_key(|p| (p.y, p.x));
        found
    }

    /// Where the player can get to in order to shove a boulder, with the step that got them there
    /// — a flood fill from where they stand, and deliberately not [`Self::bfs_from_player`].
    fn push_search(&self) -> (std::collections::HashSet<Point8>,
                              HashMap<Point8, (Point8, JoypadButton)>) {
        use std::collections::{HashSet, VecDeque};
        let boulders: HashSet<Point8> = self.boulders().into_iter().collect();
        let standable = |p: Point8| !boulders.contains(&p) && match self.tile_at(p) {
            MetaTile::Empty | MetaTile::Grass => true,
            MetaTile::Warp { .. } => self.warp_trigger(p) != WarpTrigger::StepOn,
            _ => false,
        };
        let mut seen = HashSet::from([self.player_position]);
        let mut came: HashMap<Point8, (Point8, JoypadButton)> = HashMap::new();
        let mut queue = VecDeque::from([self.player_position]);
        while let Some(at) = queue.pop_front() {
            for dir in [JoypadButton::Up, JoypadButton::Down, JoypadButton::Left, JoypadButton::Right] {
                let Some(next) = self.step(at, dir) else { continue };
                if !standable(next) || self.pair_blocked(at, next) { continue }
                // Only ask a *warp* what fires it.
                if matches!(self.tile_at(next), MetaTile::Warp { .. })
                    && self.warp_trigger(next) == WarpTrigger::HoldDirection(dir) { continue }
                if seen.insert(next) {
                    came.insert(next, (at, dir));
                    queue.push_back(next);
                }
            }
        }
        (seen, came)
    }

    /// The walk to a square a boulder can be shoved from, under `Self::push_search`'s rules
    /// rather than `route_to`'s — which is the same disagreement one layer down:
    /// `AgentState::PushingBoulder` routed with `route_to`, so even a push the menu offered could
    /// have been a walk the driver could not make, and it drops to `Idle` without a word when it
    /// cannot find one.
    pub fn route_to_push_tile(&self, dest: Point8) -> Option<Vec<JoypadButton>> {
        let (seen, came) = self.push_search();
        if !seen.contains(&dest) { return None }
        let mut route = vec![];
        let mut at = dest;
        while let Some(&(prev, dir)) = came.get(&at) {
            route.push(dir);
            at = prev;
        }
        route.reverse();
        Some(route)
    }

    /// Every one-tile shove on this map the cartridge would actually carry out, as `(boulder,
    /// direction, the square it is pushed from)`.
    pub fn boulder_pushes(&self) -> Vec<(Point8, JoypadButton, Point8)> {
        self.boulder_pushes_within(&self.push_search().0)
    }

    /// [`Self::boulder_pushes`] against a `Self::push_search` already run, which is what
    /// `actions()` calls: it wants the came-from map as well, to build each row's walk, and a
    /// second flood fill per tick is the cost `nearest_castable_water` learned about the hard
    /// way.
    pub fn boulder_pushes_within(&self, reach: &std::collections::HashSet<Point8>)
        -> Vec<(Point8, JoypadButton, Point8)> {
        let mut pushes = vec![];
        for boulder in self.boulders() {
            for dir in [JoypadButton::Up, JoypadButton::Down, JoypadButton::Left, JoypadButton::Right] {
                if self.boulder_push_refusal_inner(boulder, dir, reach).is_some() { continue }
                let Some(stand) = self.step(boulder, opposite_dir(dir)) else { continue };
                pushes.push((boulder, dir, stand));
            }
        }
        pushes
    }

    /// One square in `d` from `p`, or `None` off the map.
    fn step(&self, p: Point8, d: JoypadButton) -> Option<Point8> {
        let (dx, dy): (i32, i32) = match d {
            JoypadButton::Up => (0, -1), JoypadButton::Down => (0, 1),
            JoypadButton::Left => (-1, 0), JoypadButton::Right => (1, 0),
            _ => return None,
        };
        let (x, y) = (p.x as i32 + dx, p.y as i32 + dy);
        (x >= 0 && y >= 0 && (x as usize) < self.width && (y as usize) < self.height)
            .then(|| Point8 { x: x as u8, y: y as u8 })
    }

    /// Why the cartridge would refuse to shove the boulder at `boulder` one tile in `dir`, in a
    /// sentence the model can act on — or `None` if it would move.
    pub fn boulder_push_refusal(&self, boulder: Point8, dir: JoypadButton) -> Option<String> {
        let name = self.sprites.iter()
            .find(|s| s.name.starts_with("Boulder") && !s.hidden && s.position == boulder)
            .map(|s| s.name);
        let Some(name) = name else {
            return Some(format!(
                "There is no boulder at ({}, {}). `read_map` gives the exact position of every one \
                 on this map.", boulder.x, boulder.y));
        };
        // One flood fill for all four directions: the "and these do work" clause asks about three
        // more pushes, and each of them needs to know whether the tile it would be shoved from
        // can be walked to.
        let reach = self.push_search().0;
        let refused = |why: String| {
            // Naming the directions that do work is the half that matters.
            let ways: Vec<&str> = [(JoypadButton::Up, "up"), (JoypadButton::Down, "down"),
                                   (JoypadButton::Left, "left"), (JoypadButton::Right, "right")]
                .into_iter()
                .filter(|(d, _)| *d != dir && self.boulder_push_refusal_inner(boulder, *d, &reach).is_none())
                .map(|(_, w)| w).collect();
            let rest = match ways.as_slice() {
                // A boulder with nowhere to go is a *reset*, not a dead end, and saying only the
                // first half is how a model decides the game is broken.
                [] => "It cannot be pushed any way at all from where it is standing. Leaving this \
                       map and coming back puts every boulder on it back where it started, which is \
                       the way to undo a push that went wrong.".to_string(),
                ways => format!("It can be pushed {}.", ways.join(" or ")),
            };
            format!("{name} at ({}, {}) will not push {}: {why}. {rest}",
                boulder.x, boulder.y, push_word(dir))
        };
        self.boulder_push_refusal_inner(boulder, dir, &reach).map(refused)
    }

    /// [`Self::boulder_push_refusal`] without the sentence, so the "and these directions do work"
    /// clause can ask the same question of the other three without recursing into its own prose.
    fn boulder_push_refusal_inner(&self, boulder: Point8, dir: JoypadButton,
        reach: &std::collections::HashSet<Point8>) -> Option<String> {
        let Some(dest) = self.step(boulder, dir) else {
            return Some(format!("the edge of {} is there", self.map));
        };
        // The tile the player has to stand on to shove it, one square the other way.
        let Some(stand) = self.step(boulder, opposite_dir(dir)) else {
            return Some(format!("there is nowhere to stand on the far side, at the edge of {}", self.map));
        };
        match self.tile_at(dest) {
            // A hole is a legitimate destination — Victory Road 3F and Seafoam B3F are solved by
            // dropping a boulder through one — so a warp tile is passable here where it is not
            // for `dest_floor`'s ordinary floor.
            MetaTile::Empty | MetaTile::Grass | MetaTile::Warp { .. } => {}
            MetaTile::Sprite(who) => return Some(format!("{who} is standing in the way at ({}, {})", dest.x, dest.y)),
            other => return Some(format!("({}, {}) is {other}", dest.x, dest.y)),
        }
        if let Some(refusal) = self.boulder_push_terrain_refusal(stand, dest) {
            return Some(refusal);
        }
        // Not one of the cartridge's checks, and it has to be here anyway.
        match self.tile_at(stand) {
            MetaTile::Empty | MetaTile::Grass => {}
            // A warp entry is floor unless it fires on the step onto it.
            MetaTile::Warp { .. } if self.warp_trigger(stand) != WarpTrigger::StepOn => {}
            MetaTile::Warp { to_map, .. } => return Some(format!(
                "({}, {}) is the way to {to_map} and you would be taken there the moment you stepped \
                 on it, so there is nowhere to stand to push from that side", stand.x, stand.y)),
            MetaTile::Sprite(who) => return Some(format!(
                "{who} is standing at ({}, {}), which is the only square this can be pushed from",
                stand.x, stand.y)),
            other => return Some(format!(
                "({}, {}) is {other}, so there is nowhere to stand to push from that side",
                stand.x, stand.y)),
        }
        if stand != self.player_position && !reach.contains(&stand) {
            return Some(format!(
                "you cannot get to ({}, {}) to push from there", stand.x, stand.y));
        }
        None
    }

    /// Multi-boulder Sokoban: plan a sequence of one-tile pushes that lands *some* boulder on
    /// `switch`. Each entry is `(boulder_position_before_that_push, push_direction)`; returns
    /// `None` if no boulder can reach the switch. Boulders are the sprites named "Boulder …".
    pub fn solve_boulder_push(&self, switch: Point8) -> Option<Vec<(Point8, JoypadButton)>> {
        self.solve_boulder_push_tracking(None, switch)
    }

    /// [`Self::solve_boulder_push`], for one named boulder rather than whichever gets there
    /// first.
    pub fn solve_boulder_push_for(&self, boulder: Point8, switch: Point8)
        -> Option<Vec<(Point8, JoypadButton)>> {
        self.solve_boulder_push_tracking(Some(boulder), switch)
    }

    /// `tracked` is kept at index 0 and only the tail is canonicalised.
    fn solve_boulder_push_tracking(&self, tracked: Option<Point8>, switch: Point8)
        -> Option<Vec<(Point8, JoypadButton)>> {
        use std::collections::{HashMap, HashSet, VecDeque};
        /// Layouts explored before giving up.
        const MAX_STATES: usize = 50_000;
        // Only *visible* boulders are physically present and pushable.
        let mut boulders: Vec<Point8> = self.sprites.iter()
            .filter(|s| s.name.starts_with("Boulder") && !s.hidden)
            .map(|s| s.position).collect();
        if boulders.is_empty() { return None; }
        boulders.sort_by_key(|p| (p.y, p.x));   // canonical order, so a layout has one key
        // The tracked boulder moves to index 0 and stays out of every later sort.
        if let Some(t) = tracked {
            let at = boulders.iter().position(|b| *b == t)?;
            boulders.swap(0, at);
            boulders[1..].sort_by_key(|p| (p.y, p.x));
        }
        // `self.tile_at` reports the *live* boulder sprites as occupied, but the solver simulates
        // boulders moving — so the tile UNDER any boulder's STARTING position must count as floor
        // (the solver tracks occupancy itself).
        let initial: HashSet<Point8> = boulders.iter().copied().collect();
        let dirs = [(0i32, -1i32, JoypadButton::Up), (0, 1, JoypadButton::Down),
                    (-1, 0, JoypadButton::Left), (1, 0, JoypadButton::Right)];
        let inb = |x: i32, y: i32| x >= 0 && y >= 0 && (x as usize) < self.width && (y as usize) < self.height;
        let mv = |p: Point8, dx: i32, dy: i32| -> Option<Point8> {
            let (x, y) = (p.x as i32 + dx, p.y as i32 + dy);
            inb(x, y).then(|| Point8 { x: x as u8, y: y as u8 })
        };
        // A tile the player may STAND ON to push a boulder: ordinary floor, and also inter-map
        // warp tiles.
        let floor = |p: Point8| initial.contains(&p)
            || matches!(self.tile_at(p), MetaTile::Empty | MetaTile::Grass | MetaTile::Warp { .. });
        // A tile a BOULDER may be pushed onto: ordinary floor (or a tile vacated by a boulder),
        // or the explicit `switch` target itself — this lets the caller aim a boulder at a hole
        // tile (a `MetaTile::Warp`) to drop it to the floor below (Victory Road 3F, Seafoam B3F),
        // which normal floor rules would reject.
        let dest_floor = |p: Point8| p == switch || initial.contains(&p)
            || matches!(self.tile_at(p), MetaTile::Empty | MetaTile::Grass);
        // Tiles the player can reach from `from` with this layout's boulders as walls.
        let reach = |bs: &[Point8], from: Point8| -> HashSet<Point8> {
            let mut seen = HashSet::from([from]);
            let mut q = VecDeque::from([from]);
            while let Some(p) = q.pop_front() {
                for &(dx, dy, _) in &dirs {
                    if let Some(n) = mv(p, dx, dy) {
                        if floor(n) && !bs.contains(&n) && !self.pair_blocked(p, n) && seen.insert(n) {
                            q.push_back(n);
                        }
                    }
                }
            }
            seen
        };
        let norm = |set: &HashSet<Point8>| -> Point8 { *set.iter().min_by_key(|p| (p.y, p.x)).unwrap() };

        // An optimistic pre-filter, because an *unsolvable* pair is what costs the cap.
        let could_possibly_reach = |from: Point8| -> bool {
            let mut seen = HashSet::from([from]);
            let mut q = VecDeque::from([from]);
            while let Some(b) = q.pop_front() {
                if b == switch { return true; }
                for &(dx, dy, _) in &dirs {
                    let (Some(side), Some(dest)) = (mv(b, -dx, -dy), mv(b, dx, dy)) else { continue };
                    // The player has to be able to *stand* behind it and the boulder has to be
                    // able to *land* in front: both are properties of the terrain alone, so they
                    // hold however the other boulders are arranged.
                    if !floor(side) || !dest_floor(dest) { continue }
                    if self.boulder_push_terrain_refusal(side, dest).is_some() { continue }
                    if seen.insert(dest) { q.push_back(dest); }
                }
            }
            false
        };
        // Answered from `PLAN_CACHE` when this exact floor has been asked before, which on a 20
        // ms tick loop is almost always.
        let start_reach = reach(&boulders, self.player_position);
        let key = PlanKey {
            map: self.map,
            walkable: self.walkable_bits().to_vec(),
            layout: boulders.clone(),
            component: norm(&start_reach),
            tracked,
            target: switch,
        };
        if let Some(hit) = PLAN_CACHE.with(|c| c.borrow().get(&key).cloned()) { return hit }

        // The search proper, in a closure so that both of its exits land in the cache below
        // rather than each remembering to.
        let search = || {
        // Inside the closure, so that a "no" is cached too.
        match tracked {
            // One named boulder: it alone has to be able to get there.
            Some(t) => if !could_possibly_reach(t) { return None },
            // Any boulder will do, so the floor is hopeless only if none of them can.
            None => if !boulders.iter().any(|b| could_possibly_reach(*b)) { return None },
        }

        type Key = (Vec<Point8>, Point8);           // (boulder layout, player component)
        // The component is only known once a state is popped (it needs a flood fill), so dedup
        // happens at pop time and the queue carries the parent link to record on first arrival.
        let mut visited: HashSet<Key> = HashSet::new();
        let mut came: HashMap<Key, (Key, Point8, JoypadButton)> = HashMap::new();
        let mut q: VecDeque<(Vec<Point8>, Point8, Option<(Key, Point8, JoypadButton)>)> =
            VecDeque::from([(boulders.clone(), self.player_position, None)]);
        while let Some((bs, player_at, parent)) = q.pop_front() {
            let key: Key = (bs.clone(), norm(&reach(&bs, player_at)));
            if !visited.insert(key.clone()) { continue; }
            if let Some(link) = parent { came.insert(key.clone(), link); }
            let solved = match tracked {
                Some(_) => bs[0] == switch,
                None => bs.contains(&switch),
            };
            if solved {
                if std::env::var("BOULDER_DEBUG").is_ok() {
                    eprintln!("  switch {switch}: solved after exploring {} layouts", visited.len());
                }
                let mut pushes = vec![];
                let mut cur = key;
                while let Some((prev, from, dir)) = came.get(&cur).cloned() {
                    pushes.push((from, dir));
                    cur = prev;
                }
                pushes.reverse();
                return Some(pushes);
            }
            if visited.len() >= MAX_STATES {
                if std::env::var("BOULDER_DEBUG").is_ok() {
                    eprintln!("  switch {switch}: gave up at the {MAX_STATES}-layout cap");
                }
                break;
            }
            let r = reach(&bs, player_at);
            for (i, &b) in bs.iter().enumerate() {
                for &(dx, dy, dir) in &dirs {
                    let (Some(side), Some(dest)) = (mv(b, -dx, -dy), mv(b, dx, dy)) else { continue };
                    // The player must be able to reach the tile behind the boulder, the
                    // destination must be plain floor, and the two extra refusals pokered
                    // `CheckForCollisionWhenPushingBoulder` adds — a staircase, and an elevation
                    // boundary — must not apply.
                    if !r.contains(&side) || bs.contains(&dest) || !dest_floor(dest) { continue; }
                    // `side`, not `b` — the cartridge tests the player's tile against the
                    // destination, and the staircase rule with it.
                    if self.boulder_push_terrain_refusal(side, dest).is_some() { continue; }
                    let mut next = bs.clone();
                    next[i] = dest;
                    match tracked {
                        Some(_) => next[1..].sort_by_key(|p| (p.y, p.x)),
                        None => next.sort_by_key(|p| (p.y, p.x)),
                    }
                    // After the push the player stands on the boulder's old tile.
                    q.push_back((next, b, Some((key.clone(), b, dir))));
                }
            }
        }
        if std::env::var("BOULDER_DEBUG").is_ok() {
            eprintln!("  switch {switch}: NO SOLUTION after {} layouts (boulders {:?})",
                visited.len(), boulders.iter().map(|p| (p.x, p.y)).collect::<Vec<_>>());
        }
        None
        };
        let answer = search();
        PLAN_CACHE.with(|c| {
            let mut cache = c.borrow_mut();
            if cache.len() >= PLAN_CACHE_CAP { cache.clear(); }
            cache.insert(key, answer.clone());
        });
        answer
    }

    /// Two bits per tile in reading order: may the player stand here, may a boulder land here.
    fn walkable_bits(&self) -> &[u64] {
        self.walkable_cache.get_or_init(|| {
        let mut bits = vec![0u64; (self.width * self.height * 2).div_ceil(64)];
        for y in 0..self.height {
            for x in 0..self.width {
                let at = Point8 { x: x as u8, y: y as u8 };
                let tile = self.tile_at(at);
                let stand = matches!(tile, MetaTile::Empty | MetaTile::Grass | MetaTile::Warp { .. });
                let land = matches!(tile, MetaTile::Empty | MetaTile::Grass);
                let base = (x + y * self.width) * 2;
                if stand { bits[base / 64] |= 1 << (base % 64); }
                if land { bits[(base + 1) / 64] |= 1 << ((base + 1) % 64); }
            }
        }
        bits
        })
    }

    /// The shortest walking route (button sequence) from the player to an arbitrary reachable
    /// tile, or `None` if unreachable. Used to position the player next to a boulder before a
    /// Strength push (the standard `actions()` routes only target *typed* tiles, not arbitrary
    /// floor positions).
    pub fn route_to(&self, dest: Point8) -> Option<Vec<JoypadButton>> {
        let (dist, came_from) = self.bfs_from_player();
        if !dist.contains_key(&dest) { return None; }
        let mut route = vec![];
        let mut pos = dest;
        while let Some(&(prev, dir)) = came_from.get(&pos) {
            route.push(dir);
            pos = prev;
        }
        route.reverse();
        Some(route)
    }

    /// One `Self::bfs_from_player`, for a caller that is about to ask
    /// [`Self::route_to_face_within`] about many targets. See that method for why the loop must
    /// not run its own search per target.
    pub fn search_for_faces(&self) -> (HashMap<Point8, u32>, HashMap<Point8, (Point8, JoypadButton)>) {
        self.bfs_from_player()
    }

    /// Search from `player_position` outward.
    fn bfs_from_player(&self) -> (HashMap<Point8, u32>, HashMap<Point8, (Point8, JoypadButton)>) {
        let (dist, _, came_from) = self.search_from_player();
        (dist, came_from)
    }

    /// [`Self::bfs_from_player`] plus the step count along each tile's chosen route, for the one
    /// caller that wants distance in the walking sense rather than in the priced one.
    fn search_from_player(&self)
        -> (HashMap<Point8, u32>, HashMap<Point8, u32>, HashMap<Point8, (Point8, JoypadButton)>) {
        use std::collections::{HashMap, HashSet, VecDeque};

        let mut dist: HashMap<Point8, u32> = HashMap::new();
        let mut steps: HashMap<Point8, u32> = HashMap::new();
        let mut came_from: HashMap<Point8, (Point8, JoypadButton)> = HashMap::new();
        let mut settled: HashSet<Point8> = HashSet::new();
        let mut buckets: Vec<VecDeque<Point8>> = vec![VecDeque::new()];

        // If the player is standing on an arrow tile (mid-slide), the forced movement will carry
        // them to its rest destination — start the search from there.
        let start = self.resolve_spinner(self.player_position);
        dist.insert(start, 0);
        steps.insert(start, 0);
        buckets[0].push_back(start);

        macro_rules! relax {
            ($from:expr, $to:expr, $dir:expr, $edge:expr) => {{
                let price = dist[&$from] + $edge;
                if !settled.contains(&$to) && dist.get(&$to).is_none_or(|&d| price < d) {
                    dist.insert($to, price);
                    steps.insert($to, steps[&$from] + 1);
                    came_from.insert($to, ($from, $dir));
                    true
                } else { false }
            }};
        }

        // Buckets are grown on demand: the highest price any tile can carry is one step per
        // square plus one mount per land→water boundary crossed, which is bounded but not worth
        // computing.
        fn push(buckets: &mut Vec<VecDeque<Point8>>, price: u32, p: Point8) {
            let i = price as usize;
            if buckets.len() <= i { buckets.resize(i + 1, VecDeque::new()); }
            buckets[i].push_back(p);
        }

        let mut bucket = 0usize;
        while bucket < buckets.len() {
            while let Some(pos) = buckets[bucket].pop_front() {
                // A stale copy: this tile was queued at this price and then found more cheaply,
                // or it has already been expanded.
                if dist[&pos] != bucket as u32 || !settled.insert(pos) { continue; }
                let neighbors = [
                    (JoypadButton::Up,    Point8 { x: pos.x,                    y: pos.y.wrapping_sub(1) }),
                    (JoypadButton::Down,  Point8 { x: pos.x,                    y: pos.y.wrapping_add(1) }),
                    (JoypadButton::Left,  Point8 { x: pos.x.wrapping_sub(1),    y: pos.y                 }),
                    (JoypadButton::Right, Point8 { x: pos.x.wrapping_add(1),    y: pos.y                 }),
                ];
                let here = self.meta_tiles[pos.x as usize + pos.y as usize * self.width];
                for (dir, nb) in neighbors {
                    if nb.x as usize >= self.width || nb.y as usize >= self.height { continue; }

                    // Intra-map teleporter (the Saffron Gym warp maze): stepping onto `nb` warps
                    // the player to `to_position` on *this same map*.
                    if let MetaTile::Warp { to_map, to_position }
                        = self.meta_tiles[nb.x as usize + nb.y as usize * self.width]
                        && to_map == self.map
                    {
                        if (to_position.x as usize) < self.width && (to_position.y as usize) < self.height
                            && relax!(pos, to_position, dir, 1)
                        {
                            push(&mut buckets, dist[&to_position], to_position);
                        }
                        // The pad itself is deliberately *not* relaxed, so it never gets a `dist`
                        // entry of its own and never becomes a place a route may end.
                        continue;
                    }

                    if settled.contains(&nb) { continue; }

                    // Arrow (spinner) tile: stepping onto `nb` hands control to the game, which
                    // slides the player to a fixed destination.
                    if self.spinners.contains_key(&nb) {
                        let dest = self.resolve_spinner(nb);
                        if relax!(pos, dest, dir, 1) { push(&mut buckets, dist[&dest], dest); }
                        continue;
                    }

                    let tile = &self.meta_tiles[nb.x as usize + nb.y as usize * self.width];

                    if let MetaTile::Jump(jump_dir) = tile {
                        // The player never stands on a Jump tile — they either jump over it (one
                        // button press, two tiles of movement) or are blocked.
                        let can_jump = matches!((dir, jump_dir),
                            (JoypadButton::Down,  JumpDirection::South) |
                            (JoypadButton::Left,  JumpDirection::West)  |
                            (JoypadButton::Right, JumpDirection::East)
                        );
                        if can_jump {
                            if let Some(landing) = step_one(nb, dir, self.width, self.height) {
                                let landing_tile = &self.meta_tiles[landing.x as usize + landing.y as usize * self.width];
                                // Only record the landing if the player can actually stand on it.
                                let landing_blocked = matches!(landing_tile,
                                    MetaTile::Obstacle | MetaTile::Sprite(_) | MetaTile::Water |
                                    MetaTile::ConnectionWater(_) | MetaTile::Jump(_)
                                );
                                if !landing_blocked && relax!(pos, landing, dir, 1) {
                                    push(&mut buckets, dist[&landing], landing);
                                }
                            }
                        }
                    } else {
                        // Elevation boundary: the player cannot step between certain tile pairs
                        // even though both are passable (e.g. Cavern $20↔$05).
                        if self.pair_blocked(pos, nb) { continue; }
                        // Getting ON the water is refused from certain shore tiles (Seafoam B4F's
                        // (7,11), where the current is "much too fast").
                        if self.no_surf_mount.contains(&pos)
                            && matches!(tile, MetaTile::Water | MetaTile::ConnectionWater(_))
                            && !matches!(here, MetaTile::Water | MetaTile::ConnectionWater(_))
                        {
                            continue;
                        }
                        // Getting on the water is the one step that is not one step.
                        let mounting = !matches!(here, MetaTile::Water | MetaTile::ConnectionWater(_))
                            && matches!(tile, MetaTile::Water | MetaTile::ConnectionWater(_));
                        let edge = if mounting { 1 + SURF_MOUNT_COST } else { 1 };
                        if !relax!(pos, nb, dir, edge) { continue; }
                        // Warp and Connection tiles are terminal: the player can reach one but
                        // cannot walk *through* it, because stepping onto it fires the
                        // transition.
                        if self.is_pass_through(*tile) {
                            push(&mut buckets, dist[&nb], nb);
                        }
                    }
                }
            }
            bucket += 1;
        }
        (dist, steps, came_from)
    }

    /// True if the player can stand on this tile and walk on from it — the predicate
    /// [`Self::bfs_from_player`] queues a relaxed neighbour on.
    fn is_pass_through(&self, tile: MetaTile) -> bool {
        let surfable_water = self.can_surf && matches!(tile, MetaTile::Water);
        surfable_water || !matches!(tile,
            MetaTile::Obstacle | MetaTile::Sprite(_) | MetaTile::Water
            | MetaTile::ConnectionWater(_) | MetaTile::Counter | MetaTile::CutTree
            | MetaTile::Warp { .. } | MetaTile::Connection { .. })
    }

    /// Fixed PC-tile coordinates on this map — see [`pc_locations_for`].
    fn pc_locations(&self) -> &'static [Point8] {
        pc_locations_for(self.map)
    }

    /// Does this map draw tall grass anywhere, reachable or not?
    pub fn has_grass_tiles(&self) -> bool {
        self.meta_tiles.iter().any(|tile| *tile == MetaTile::Grass)
    }

    /// Fixed hidden-object sites on this map — see [`hidden_objects_for`].
    fn hidden_objects(&self) -> &'static [HiddenObjectSite] {
        hidden_objects_for(self.map)
    }

    /// Every row this map offers, from where the player is standing.
    pub fn actions(&self) -> Vec<OverworldAction> {
        if !self.position_settled {
            return vec![];
        }
        let (full_dist,     full_from)     = self.bfs_from_player();

        // Reconstruct the step sequence from the given came_from back-pointers.
        let reconstruct = |dest: Point8, came_from: &HashMap<Point8, (Point8, JoypadButton)>| -> Vec<JoypadButton> {
            let mut route = vec![];
            let mut pos = dest;
            // Walk back to the BFS root (the node with no predecessor).
            while let Some(&(prev, dir)) = came_from.get(&pos) {
                route.push(dir);
                pos = prev;
            }
            route.reverse();
            route
        };

        let best_dist_from = |p: &Point8| -> Option<(&HashMap<Point8, u32>, &HashMap<Point8, (Point8, JoypadButton)>)> {
            if full_dist.contains_key(p) {
                Some((&full_dist, &full_from))
            } else {
                None
            }
        };

        // Find the nearest tile matching `pred`
        let nearest = |pred: &dyn Fn(&MetaTile) -> bool| -> Option<(MetaTile, Point8)> {
            self.meta_tiles.iter()
                .enumerate()
                .filter(|(_, t)| pred(t))
                .map(|(i, t)| (*t, Point8 { x: (i % self.width) as u8, y: (i / self.width) as u8 }))
                .filter(|(_, p)| best_dist_from(p).is_some())
                .min_by_key(|(_, p)| best_dist_from(p).unwrap().0[p])
        };

        let mut actions = vec![];

        for (warp_to_map, warp_to_pos) in &self.warp_targets {
            // A teleport pad whose landing is underfoot is not a row.
            if *warp_to_map == self.map && self.player_position == *warp_to_pos { continue }

            let Some((tile, dest)) = nearest(&|t| matches!(t, MetaTile::Warp { to_map, to_position }
                if to_map == warp_to_map && to_position == warp_to_pos)) else { continue };
            // A warp entry the cartridge will not open is worse than no row.
            let trigger = self.warp_trigger(dest);
            if trigger == WarpTrigger::Impossible { continue }
            let (_, came_from) = best_dist_from(&dest).unwrap();
            let mut route = reconstruct(dest, came_from);

            let enter_dir = match trigger {
                // The cartridge has told us which way to face; nothing else will do, and the map
                // edge is a worse guess than the answer.
                WarpTrigger::HoldDirection(dir) => dir,
                _ => if dest.x == 0 { JoypadButton::Left }
                else if dest.x == (self.width - 1) as u8 { JoypadButton::Right }
                else if dest.y == 0 { JoypadButton::Up }
                else if dest.y == (self.height - 1) as u8 { JoypadButton::Down }
                else { *route.last().unwrap_or(&JoypadButton::Up) },
            };

            if route.is_empty() {
                match trigger {
                    // Surfing, the held button does nothing, and this is the cartridge's own
                    // branch rather than a guess.
                    WarpTrigger::HoldDirection(dir) if self.surfing || !self.standing_on_warp => {
                        route.push(opposite_dir(dir));
                        route.push(dir);
                    }
                    // One held button, not a step off and a step back.
                    WarpTrigger::HoldDirection(dir) => route.push(dir),
                    // A door tile warps on the step onto it and needs no direction, so step off
                    // to a genuinely walkable neighbour and step back.
                    WarpTrigger::StepOn | WarpTrigger::Impossible | WarpTrigger::Unknown => {
                        let step_off = self.walkable_neighbor_dir(dest).unwrap_or_else(|| opposite_dir(enter_dir));
                        route.push(step_off);
                        route.push(opposite_dir(step_off));
                    }
                }
            } else {
                route.push(enter_dir);
            }
            actions.push(OverworldAction { map: self.map, origin: self.player_position, destination: dest, tile, route });
        }

        // 2.
        for sprite in self.sprites.iter().filter(|s| !s.hidden) {
            let sp = sprite.position;
            // Direct adjacent positions (player is one tile from sprite).
            let direct: [(PlayerFacingDirection, Point8); 4] = [
                (PlayerFacingDirection::Down,  Point8 { x: sp.x,                   y: sp.y.saturating_sub(1) }),
                (PlayerFacingDirection::Up,    Point8 { x: sp.x,                   y: sp.y + 1               }),
                (PlayerFacingDirection::Right, Point8 { x: sp.x.saturating_sub(1), y: sp.y                   }),
                (PlayerFacingDirection::Left,  Point8 { x: sp.x + 1,               y: sp.y                   }),
            ];
            // Counter-mediated positions: if the tile adjacent to the sprite is a Counter
            // (talking-over tile), also add the position one more step further away.
            let counter_extra: Vec<(PlayerFacingDirection, Point8)> = direct.iter()
                .filter_map(|(face_dir, adj)| {
                    let ax = adj.x as usize;
                    let ay = adj.y as usize;
                    if ax >= self.width || ay >= self.height { return None; }
                    if self.meta_tiles[ax + ay * self.width] != MetaTile::Counter { return None; }
                    let over = match face_dir {
                        PlayerFacingDirection::Down  => adj.y.checked_sub(1).map(|y| Point8 { x: adj.x, y }),
                        PlayerFacingDirection::Up    => (ay + 1 < self.height).then_some(Point8 { x: adj.x, y: adj.y + 1 }),
                        PlayerFacingDirection::Right => adj.x.checked_sub(1).map(|x| Point8 { x, y: adj.y }),
                        PlayerFacingDirection::Left  => (ax + 1 < self.width).then_some(Point8 { x: adj.x + 1, y: adj.y }),
                    };
                    over.map(|p| (*face_dir, p))
                })
                .collect();

            let Some((face_dir, dest)) = direct.iter().chain(counter_extra.iter())
                .filter(|(_, p)| {
                    (p.x as usize) < self.width && (p.y as usize) < self.height
                    && matches!(self.meta_tiles[p.x as usize + p.y as usize * self.width], MetaTile::Empty)
                    && best_dist_from(p).is_some()
                })
                .min_by_key(|(_, p)| best_dist_from(p).unwrap().0[p])
                .copied()
            else { continue };

            let (_, came_from) = best_dist_from(&dest).unwrap();
            let mut route = reconstruct(dest, came_from);
            let face_button: JoypadButton = face_dir.into();
            if route.is_empty() {
                if face_dir != self.player_direction { route.push(face_button); }
            } else if route.last() != Some(&face_button) {
                route.push(face_button);
            }
            route.push(JoypadButton::A);
            actions.push(OverworldAction { map: self.map, origin: self.player_position, destination: dest, tile: MetaTile::Sprite(sprite.name), route });
        }

        // 3.
        for to_map in &self.connection_targets {
            let by_land = nearest(&|t| match t {
                MetaTile::Connection { to_map: m, .. } => m == to_map,
                _ => false,
            });
            let by_water = nearest(&|t| match t {
                MetaTile::ConnectionWater(m) => self.can_surf && m == to_map,
                _ => false,
            });
            for (tile, dest) in [by_land, by_water].into_iter().flatten() {
                let (_, came_from) = best_dist_from(&dest).unwrap();
                let mut route = reconstruct(dest, came_from);

                let enter_dir = if dest.y == 0 { JoypadButton::Up }
                    else if dest.y == (self.height - 1) as u8 { JoypadButton::Down }
                    else if dest.x == 0 { JoypadButton::Left }
                    else { JoypadButton::Right };
                route.push(enter_dir);
                actions.push(OverworldAction {
                    map: self.map,
                    origin: self.player_position,
                    destination: dest,
                    tile,
                    route
                });
            }
        }

        // 4.
        if self.has_grass_encounters && let Some((_, dest)) = nearest(&|t| *t == MetaTile::Grass) {
            let route = reconstruct(dest, &full_from);
            actions.push(OverworldAction { map: self.map, origin: self.player_position, destination: dest, tile: MetaTile::Grass, route });
        }

        // 6.
        if let Some(rod) = self.best_rod
            && crate::pokemon::postgame::fishing::tileset_holds_water(self.tileset)
            && let Some(water) = crate::pokemon::postgame::fishing::nearest_castable_water(self)
        {
            // This is [`Self::route_to_face_within`]'s body inlined, and that the two agree is
            // load-bearing rather than incidental.
            let adj: [(PlayerFacingDirection, Point8); 4] = [
                (PlayerFacingDirection::Down,  Point8 { x: water.x,                   y: water.y.saturating_sub(1) }),
                (PlayerFacingDirection::Up,    Point8 { x: water.x,                   y: water.y + 1               }),
                (PlayerFacingDirection::Right, Point8 { x: water.x.saturating_sub(1), y: water.y                   }),
                (PlayerFacingDirection::Left,  Point8 { x: water.x + 1,               y: water.y                   }),
            ];
            if let Some((face_dir, dest)) = adj.into_iter()
                .filter(|(_, p)| {
                    (p.x as usize) < self.width && (p.y as usize) < self.height
                    && matches!(self.meta_tiles[p.x as usize + p.y as usize * self.width], MetaTile::Empty)
                    && best_dist_from(p).is_some()
                })
                .min_by_key(|(_, p)| best_dist_from(p).unwrap().0[p])
            {
                let (_, came_from) = best_dist_from(&dest).unwrap();
                let mut route = reconstruct(dest, came_from);
                let face_button: JoypadButton = face_dir.into();
                if route.is_empty() {
                    if face_dir != self.player_direction { route.push(face_button); }
                } else if route.last() != Some(&face_button) {
                    route.push(face_button);
                }
                actions.push(OverworldAction { map: self.map, origin: self.player_position,
                    destination: dest, tile: MetaTile::Fish { rod }, route });
            }
        }

        // 5.
        for &pc in self.pc_locations() {
            // The only usable approach is from directly below, facing up.
            let dest = Point8 { x: pc.x, y: pc.y + 1 };
            if (dest.x as usize) >= self.width || (dest.y as usize) >= self.height { continue }
            if !matches!(self.meta_tiles[dest.x as usize + dest.y as usize * self.width], MetaTile::Empty) { continue }
            let Some((_, came_from)) = best_dist_from(&dest) else { continue };
            let mut route = reconstruct(dest, came_from);
            let face_button = JoypadButton::Up;
            let face_dir = PlayerFacingDirection::Up;
            if route.is_empty() {
                if face_dir != self.player_direction { route.push(face_button); }
            } else if route.last() != Some(&face_button) {
                route.push(face_button);
            }
            route.push(JoypadButton::A);
            actions.push(OverworldAction { map: self.map, origin: self.player_position, destination: dest, tile: MetaTile::Pc, route });
        }

        // 5b.
        for (index, site) in self.hidden_objects().iter().enumerate() {
            if site.object == HiddenObject::CellSeparator && !self.bill_cell_separator { continue }
            // Numbered within this map's table, over the whole table rather than per object kind,
            // so a bin's ordinal is its `wGymTrashCanIndex` plus one and a row's number never
            // shifts because something unrelated was added beside it.
            let ordinal = index as u8 + 1;
            // Below/above/left/right, each with the button that ends up facing the object.
            let approaches: [(Option<Point8>, JoypadButton, PlayerFacingDirection); 4] = [
                (site.at.y.checked_add(1).map(|y| Point8 { x: site.at.x, y }), JoypadButton::Up,    PlayerFacingDirection::Up),
                (site.at.y.checked_sub(1).map(|y| Point8 { x: site.at.x, y }), JoypadButton::Down,  PlayerFacingDirection::Down),
                (site.at.x.checked_add(1).map(|x| Point8 { x, y: site.at.y }), JoypadButton::Left,  PlayerFacingDirection::Left),
                (site.at.x.checked_sub(1).map(|x| Point8 { x, y: site.at.y }), JoypadButton::Right, PlayerFacingDirection::Right),
            ];
            let best = approaches
                .into_iter()
                .filter(|(_, _, dir)| site.facing.is_none_or(|required| required == *dir))
                .filter_map(|(dest, button, dir)| {
                    let dest = dest?;
                    if (dest.x as usize) >= self.width || (dest.y as usize) >= self.height { return None }
                    if !matches!(self.meta_tiles[dest.x as usize + dest.y as usize * self.width], MetaTile::Empty) { return None }
                    let (distances, came_from) = best_dist_from(&dest)?;
                    Some((*distances.get(&dest)?, dest, button, dir, came_from))
                })
                .min_by_key(|(distance, dest, ..)| (*distance, dest.y, dest.x));
            let Some((_, dest, face_button, face_dir, came_from)) = best else { continue };
            let mut route = reconstruct(dest, came_from);
            if route.is_empty() {
                if face_dir != self.player_direction { route.push(face_button); }
            } else if route.last() != Some(&face_button) {
                route.push(face_button);
            }
            route.push(JoypadButton::A);
            actions.push(OverworldAction { map: self.map, origin: self.player_position, destination: dest, tile: MetaTile::Switch { object: site.object, ordinal }, route });
        }

        // 6.
        let cut_trees: Vec<Point8> = match self.can_cut {
            false => Vec::new(),
            true => self.meta_tiles.iter().enumerate()
                .filter(|(_, t)| **t == MetaTile::CutTree)
                .map(|(i, _)| Point8 { x: (i % self.width) as u8, y: (i / self.width) as u8 })
                .collect(),
        };
        for tree in cut_trees {
            let adj: [(PlayerFacingDirection, Point8); 4] = [
                (PlayerFacingDirection::Down,  Point8 { x: tree.x,                   y: tree.y.saturating_sub(1) }),
                (PlayerFacingDirection::Up,    Point8 { x: tree.x,                   y: tree.y + 1               }),
                (PlayerFacingDirection::Right, Point8 { x: tree.x.saturating_sub(1), y: tree.y                   }),
                (PlayerFacingDirection::Left,  Point8 { x: tree.x + 1,               y: tree.y                   }),
            ];
            let Some((face_dir, dest)) = adj.iter()
                .filter(|(_, p)| {
                    (p.x as usize) < self.width && (p.y as usize) < self.height
                    && matches!(self.meta_tiles[p.x as usize + p.y as usize * self.width], MetaTile::Empty)
                    && best_dist_from(p).is_some()
                })
                .min_by_key(|(_, p)| best_dist_from(p).unwrap().0[p])
                .copied()
            else { continue };
            let (_, came_from) = best_dist_from(&dest).unwrap();
            let mut route = reconstruct(dest, came_from);
            let face_button: JoypadButton = face_dir.into();
            if route.is_empty() {
                if face_dir != self.player_direction { route.push(face_button); }
            } else if route.last() != Some(&face_button) {
                route.push(face_button);
            }
            actions.push(OverworldAction { map: self.map, origin: self.player_position,
                destination: dest, tile: MetaTile::Cut { at: tree }, route });
        }

        // 7.
        if self.can_strength && self.sprites.iter().any(|s| !s.hidden && s.name.starts_with("Boulder")) {
            // The row's walk comes from [`Self::push_search`] rather than from the BFS above, and
            // it has to: the routing BFS will not expand through a warp tile, and the square
            // Victory Road's puzzle has to be pushed from is one.
            let (reach, came) = self.push_search();

            // The Strength rows name the goal, not the shove: put a boulder on a switch, or into
            // a hole.
            let targets = self.strength_switches.iter().map(|at| (*at, false))
                .chain(self.holes.iter().map(|at| (*at, true)));
            // One row per target, not one per (boulder, target) pair — and that is a measured
            // retreat rather than a preference.
            for (at, hole) in targets {
                // Already done: a boulder is sitting on it, so there is no decision left here.
                if self.boulders().contains(&at) { continue }
                // Is this target reachable by *anything*?
                if self.solve_boulder_push(at).is_none() { continue }
                // Then the *nearest* boulder that can actually do it, not whichever the planner
                // reached for first.
                let mut candidates = self.boulders();
                candidates.sort_by_key(|b| (b.x as i32 - at.x as i32).abs() + (b.y as i32 - at.y as i32).abs());
                let Some((which, plan)) = candidates.into_iter()
                    .find_map(|b| self.solve_boulder_push_for(b, at).map(|plan| (b, plan)))
                    else { continue };
                let Some((boulder, push)) = plan.into_iter().next() else { continue };
                let Some(stand) = self.step(boulder, opposite_dir(push)) else { continue };
                if !reach.contains(&stand) { continue }
                // The walk is to the *first* push of the plan; the driver re-plans from there and
                // keeps going, so this route is what the row promises rather than the whole
                // solution.
                let mut route = reconstruct(stand, &came);
                if route.is_empty() {
                    let facing: JoypadButton = self.player_direction.into();
                    if facing != push { route.push(push); }
                } else if route.last() != Some(&push) {
                    route.push(push);
                }
                actions.push(OverworldAction { map: self.map, origin: self.player_position,
                    destination: stand, tile: MetaTile::BoulderGoal { boulder: which, at, hole }, route });
            }
        }

        actions.sort();
        actions
    }

    /// What it takes to make the warp entry at `at` actually fire, as the cartridge decides it.
    pub fn warp_trigger(&self, at: Point8) -> WarpTrigger {
        // A script that cancels the warp beats every tile test below, because it runs after them:
        // the staircase really is a step-on warp and the cartridge really does undo it.
        if self.script_cancelled_warps.contains(&at) {
            return WarpTrigger::Impossible;
        }
        let Some(&here) = self.raw_tile_ids.get(at.x as usize + at.y as usize * self.width) else {
            return WarpTrigger::Impossible;
        };
        if self.tileset.warp_tile_ids().contains(&here)
            // …or the other table that does the same job: warp pads and floor holes, read by
            // `IsPlayerStandingOnWarpPadOrHole` rather than by
            // `IsPlayerStandingOnDoorTileOrWarpTile`.
            || self.tileset.warp_pad_and_hole_tile_ids().contains(&here)
        {
            return WarpTrigger::StepOn;
        }
        // `ExtraWarpCheck`'s dispatch, in its own order: SS Anne 3F takes function 1 whatever its
        // tileset says, four named maps take function 2 whatever theirs says, and only then does
        // the tileset decide.
        let reads_the_tile_in_front = match self.map {
            Map::SSAnne3F => false,
            Map::RocketHideoutB1F | Map::RocketHideoutB2F | Map::RocketHideoutB4F
            | Map::RockTunnel1F => true,
            _ => self.tileset.warp_check_reads_the_tile_in_front(),
        };
        if !reads_the_tile_in_front {
            // `IsPlayerFacingEdgeOfMap`: at the edge, facing out.
            let out = if at.y == 0 { Some(PlayerFacingDirection::Up) }
                else if at.y as usize == self.height.saturating_sub(1) { Some(PlayerFacingDirection::Down) }
                else if at.x == 0 { Some(PlayerFacingDirection::Left) }
                else if at.x as usize == self.width.saturating_sub(1) { Some(PlayerFacingDirection::Right) }
                else { None };
            return match out {
                Some(facing) => WarpTrigger::HoldDirection(facing.into()),
                None => WarpTrigger::Impossible,
            };
        }
        // `IsWarpTileInFrontOfPlayer`: the tile in front, against the list for the way you face.
        let mut looks_off_the_map = false;
        for (facing, dx, dy) in [
            (PlayerFacingDirection::Up,    0i32, -1i32),
            (PlayerFacingDirection::Down,  0,  1),
            (PlayerFacingDirection::Left, -1,  0),
            (PlayerFacingDirection::Right, 1,  0),
        ] {
            let (x, y) = (at.x as i32 + dx, at.y as i32 + dy);
            if x < 0 || y < 0 || x as usize >= self.width || y as usize >= self.height {
                looks_off_the_map = true;
                continue;
            }
            let front = self.raw_tile_ids[x as usize + y as usize * self.width];
            if crate::pokemon::map_header::TileSetId::warp_carpet_tile_ids(facing).contains(&front) {
                return WarpTrigger::HoldDirection(facing.into());
            }
        }
        match looks_off_the_map {
            true => WarpTrigger::Unknown,
            false => WarpTrigger::Impossible,
        }
    }

    /// True when `row` is missing from [`Self::actions`] only because somebody is standing in the
    /// way: put everybody except the row's own subject back where the map says the floor is, and
    /// the row comes back.
    pub fn row_blocked_by_people(&self, row: MetaTile) -> bool {
        let subject = match row {
            MetaTile::Sprite(name) => Some(name),
            _ => None,
        };
        let lift: Vec<(Point8, MetaTile)> = self.underfoot.iter().copied()
            .filter(|(p, _)| match self.tile_at(*p) {
                // A boulder is a sprite and is not a person.
                MetaTile::Sprite(who) => Some(who) != subject && !who.starts_with("Boulder"),
                _ => true,
            })
            .collect();
        if lift.is_empty() {
            return false;
        }
        let mut cleared = self.clone();
        // The cache is `walkable_bits`' answer for *this* map, and the whole point here is a
        // different one.
        cleared.walkable_cache = std::cell::OnceCell::new();
        for (at, was) in lift {
            cleared.meta_tiles[at.x as usize + at.y as usize * cleared.width] = was;
        }
        cleared.warp_targets = warp_targets_of(&cleared.meta_tiles);
        cleared.connection_targets = connection_targets_of(&cleared.meta_tiles);
        cleared.actions().iter().any(|action| action.tile.is_same_row_as(&row))
    }

    /// Every distinct way off this map into `to_map`, one [`Crossing`] per run of touching edge
    /// tiles, nearest-reachable first and then in reading order.
    pub fn crossings(&self, to_map: Map) -> Vec<Crossing> {
        use std::collections::{HashSet, VecDeque};
        let (dist, _) = self.bfs_from_player();
        let at = |i: usize| Point8 { x: (i % self.width) as u8, y: (i / self.width) as u8 };
        let landing = |p: Point8| match self.meta_tiles[p.x as usize + p.y as usize * self.width] {
            MetaTile::Connection { to_map: m, to_position } if m == to_map => Some(to_position),
            _ => None,
        };
        let all: HashSet<Point8> = self.meta_tiles.iter().enumerate()
            .filter(|(_, t)| matches!(t, MetaTile::Connection { to_map: m, .. } if *m == to_map))
            .map(|(i, _)| at(i))
            .collect();

        let mut seen: HashSet<Point8> = HashSet::new();
        let mut crossings = vec![];
        // Flood one run at a time over 4-neighbours, so a wall between two stretches of the same
        // border strip splits them and a diagonal notch does not.
        for &start in &all {
            if !seen.insert(start) { continue }
            let mut run = vec![start];
            let mut queue = VecDeque::from([start]);
            while let Some(p) = queue.pop_front() {
                for next in [
                    p.y.checked_sub(1).map(|y| Point8 { x: p.x, y }),
                    (p.y as usize + 1 < self.height).then(|| Point8 { x: p.x, y: p.y + 1 }),
                    p.x.checked_sub(1).map(|x| Point8 { x, y: p.y }),
                    (p.x as usize + 1 < self.width).then(|| Point8 { x: p.x + 1, y: p.y }),
                ].into_iter().flatten() {
                    if all.contains(&next) && seen.insert(next) {
                        run.push(next);
                        queue.push_back(next);
                    }
                }
            }
            // The tile that names the run: the reachable one nearest the player if the run can be
            // reached at all, otherwise the first in reading order.
            let named = run.iter().copied()
                .filter(|p| dist.contains_key(p))
                .min_by_key(|p| (dist[p], p.y, p.x))
                .or_else(|| run.iter().copied().min_by_key(|p| (p.y, p.x)));
            let Some(named) = named else { continue };
            let Some(to_position) = landing(named) else { continue };
            crossings.push(Crossing {
                at: named,
                to_position,
                reachable: run.iter().any(|p| dist.contains_key(p)),
                tiles: run.len(),
            });
        }
        crossings.sort_by_key(|c| (
            !c.reachable,
            dist.get(&c.at).copied().unwrap_or(u32::MAX),
            std::cmp::Reverse(c.tiles),
            c.at.y,
            c.at.x,
        ));
        crossings
    }

    /// What the region the player can walk in ends on: the kinds of impassable tile that touch
    /// it, commonest first, as noun phrases fit to drop into a sentence.
    pub fn boundary_blockers(&self) -> Vec<&'static str> {
        let mut counts: Vec<(&'static str, usize)> = vec![];
        for at in self.reachable_tiles() {
            let noun = match self.tile_at(at) {
                MetaTile::CutTree => "trees that Cut clears",
                MetaTile::Water | MetaTile::ConnectionWater(_) => "water that needs Surf",
                MetaTile::Jump(_) => "ledges, which only go one way",
                MetaTile::Sprite(_) => "people and objects standing in the gap",
                MetaTile::Counter => "counters, which are talked over rather than walked round",
                _ => continue,
            };
            match counts.iter_mut().find(|(n, _)| *n == noun) {
                Some((_, count)) => *count += 1,
                None => counts.push((noun, 1)),
            }
        }
        counts.sort_by_key(|(noun, count)| (std::cmp::Reverse(*count), *noun));
        counts.into_iter().map(|(noun, _)| noun).collect()
    }

    /// Build the action that crosses a connection to `to_map` landing at raw `to_position`, if
    /// that specific connection tile is reachable. Kept out of `actions()` (which emits only the
    /// nearest crossing per adjacent map) so `EnterMap { to_position }` can target a particular
    /// landing — e.g. to avoid a dead-end pocket at the nearest crossing (Route 13→14 row 6) —
    /// without bloating the per-step action list (which, emitted per-edge, perturbs
    /// `route_toward`/grind navigation).
    pub fn connection_action(&self, to_map: Map, to_position: Point8) -> Option<OverworldAction> {
        let (full_dist, full_from) = self.bfs_from_player();
        let (dest, tile) = self.meta_tiles.iter().enumerate()
            .filter_map(|(i, t)| match t {
                MetaTile::Connection { to_map: cm, to_position: tp } if *cm == to_map && *tp == to_position => {
                    let p = Point8 { x: (i % self.width) as u8, y: (i / self.width) as u8 };
                    full_dist.get(&p).map(|d| (*d, p, *t))
                }
                _ => None,
            })
            .min_by_key(|(d, _, _)| *d)
            .map(|(_, p, t)| (p, t))?;

        let mut route = vec![];
        let mut pos = dest;
        while let Some(&(prev, dir)) = full_from.get(&pos) { route.push(dir); pos = prev; }
        route.reverse();
        let enter_dir = if dest.y == 0 { JoypadButton::Up }
            else if dest.y == (self.height - 1) as u8 { JoypadButton::Down }
            else if dest.x == 0 { JoypadButton::Left }
            else { JoypadButton::Right };
        route.push(enter_dir);
        Some(OverworldAction { map: self.map, origin: self.player_position, destination: dest, tile, route })
    }

    /// The goal row for one named boulder onto `at`, built on demand.
    pub fn boulder_goal_action(&self, boulder: Point8, at: Point8, hole: bool)
        -> Option<OverworldAction> {
        if !self.can_strength { return None }
        if self.boulders().contains(&at) { return None }
        let (reach, came) = self.push_search();
        let plan = self.solve_boulder_push_for(boulder, at)?;
        let (push_from, push) = plan.into_iter().next()?;
        let stand = self.step(push_from, opposite_dir(push))?;
        if !reach.contains(&stand) { return None }
        let mut route = self.reconstruct_from(stand, &came);
        if route.is_empty() {
            let facing: JoypadButton = self.player_direction.into();
            if facing != push { route.push(push); }
        } else if route.last() != Some(&push) {
            route.push(push);
        }
        Some(OverworldAction {
            map: self.map, origin: self.player_position, destination: stand,
            tile: MetaTile::BoulderGoal { boulder, at, hole }, route,
        })
    }

    /// Walk back a `push_search` came-from map into the button sequence that reaches `dest`.
    fn reconstruct_from(&self, dest: Point8,
                        came: &HashMap<Point8, (Point8, JoypadButton)>) -> Vec<JoypadButton> {
        let mut route = vec![];
        let mut pos = dest;
        while let Some(&(prev, dir)) = came.get(&pos) { route.push(dir); pos = prev; }
        route.reverse();
        route
    }

    /// Route to the nearest reachable water edge into `to_map` — a `ConnectionWater` tile,
    /// crossed by Surfing off the map edge.
    pub fn water_connection_action(&self, to_map: Map) -> Option<OverworldAction> {
        let (full_dist, full_from) = self.bfs_from_player();
        let (dest, tile) = self.meta_tiles.iter().enumerate()
            .filter_map(|(i, t)| match t {
                MetaTile::ConnectionWater(m) if *m == to_map => {
                    let p = Point8 { x: (i % self.width) as u8, y: (i / self.width) as u8 };
                    full_dist.get(&p).map(|d| (*d, p, *t))
                }
                _ => None,
            })
            .min_by_key(|(d, _, _)| *d)
            .map(|(_, p, t)| (p, t))?;

        let mut route = vec![];
        let mut pos = dest;
        while let Some(&(prev, dir)) = full_from.get(&pos) { route.push(dir); pos = prev; }
        route.reverse();
        let enter_dir = if dest.y == 0 { JoypadButton::Up }
            else if dest.y == (self.height - 1) as u8 { JoypadButton::Down }
            else if dest.x == 0 { JoypadButton::Left }
            else { JoypadButton::Right };
        route.push(enter_dir);
        Some(OverworldAction { map: self.map, origin: self.player_position, destination: dest, tile, route })
    }

    /// The tile directly in front of the player (based on facing), if within bounds.
    pub fn tile_in_front(&self) -> Option<(Point8, MetaTile)> {
        let p = self.player_position;
        let front = match self.player_direction {
            PlayerFacingDirection::Up    => Point8 { x: p.x, y: p.y.checked_sub(1)? },
            PlayerFacingDirection::Down  => Point8 { x: p.x, y: p.y + 1 },
            PlayerFacingDirection::Left  => Point8 { x: p.x.checked_sub(1)?, y: p.y },
            PlayerFacingDirection::Right => Point8 { x: p.x + 1, y: p.y },
        };
        if (front.x as usize) < self.width && (front.y as usize) < self.height {
            Some((front, self.meta_tiles[front.x as usize + front.y as usize * self.width]))
        } else {
            None
        }
    }

    /// What an `A` press here would actually talk to: the tile in front, or the one *behind* it
    /// when that is a [`MetaTile::Counter`] and a person is standing there.
    pub fn interaction_in_front(&self) -> Option<(Point8, MetaTile)> {
        let (at, tile) = self.tile_in_front()?;
        if tile != MetaTile::Counter { return Some((at, tile)); }
        let over = step_one(at, self.player_direction.into(), self.width, self.height);
        match over.map(|p| (p, self.tile_at(p))) {
            Some((p, sprite @ MetaTile::Sprite(_))) => Some((p, sprite)),
            _ => Some((at, tile)),
        }
    }

    /// Route the player to a walkable (`Empty`) tile adjacent to `target` and turn to face it.
    /// Returns the button sequence (movement steps + a final turn), which is empty if the player
    /// is already adjacent and facing.
    pub fn route_to_face(&self, target: Point8) -> Option<Vec<JoypadButton>> {
        self.route_to_face_dir(target, None)
    }

    /// Like `route_to_face`, but if `required` is `Some(dir)` only the approach that ends with
    /// the player facing `dir` is considered. Needed for hidden-object switches (Pokémon Mansion
    /// statues) that only trigger when the player faces them from a specific direction —
    /// approaching from any other adjacent tile faces the wrong way and pressing A does nothing.
    pub fn route_to_face_dir(&self, target: Point8, required: Option<PlayerFacingDirection>) -> Option<Vec<JoypadButton>> {
        let (dist, came_from) = self.bfs_from_player();
        self.route_to_face_within(&dist, &came_from, target, required)
    }

    /// [`Self::route_to_face_dir`] against a search somebody else has already run.
    pub fn route_to_face_within(
        &self,
        dist: &HashMap<Point8, u32>,
        came_from: &HashMap<Point8, (Point8, JoypadButton)>,
        target: Point8,
        required: Option<PlayerFacingDirection>,
    ) -> Option<Vec<JoypadButton>> {
        let adj: [(PlayerFacingDirection, Point8); 4] = [
            (PlayerFacingDirection::Down,  Point8 { x: target.x,                   y: target.y.saturating_sub(1) }),
            (PlayerFacingDirection::Up,    Point8 { x: target.x,                   y: target.y + 1               }),
            (PlayerFacingDirection::Right, Point8 { x: target.x.saturating_sub(1), y: target.y                   }),
            (PlayerFacingDirection::Left,  Point8 { x: target.x + 1,               y: target.y                   }),
        ];
        let (face_dir, dest) = adj.into_iter()
            .filter(|(dir, _)| required.map_or(true, |r| *dir == r))
            .filter(|(_, p)| {
                (p.x as usize) < self.width && (p.y as usize) < self.height
                && matches!(self.meta_tiles[p.x as usize + p.y as usize * self.width], MetaTile::Empty)
                && dist.contains_key(p)
            })
            .min_by_key(|(_, p)| dist[p])?;
        let mut route = Vec::new();
        let mut cur = dest;
        while cur != self.player_position {
            let (prev, btn) = came_from.get(&cur)?;
            route.push(*btn);
            cur = *prev;
        }
        route.reverse();
        let face_button: JoypadButton = face_dir.into();
        if route.is_empty() {
            if face_dir != self.player_direction { route.push(face_button); }
        } else if route.last() != Some(&face_button) {
            route.push(face_button);
        }
        Some(route)
    }
}

impl Display for MetaTileMap {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        for y in 0..self.height {
            for x in 0..self.width {

                if y as u8 == self.player_position.y
                    && x as u8 == self.player_position.x {
                    // The player
                    write!(f, "P")?;
                    continue;
                }

                match self.meta_tiles[x + y * self.width] {
                    MetaTile::Empty => write!(f, "_")?,
                    MetaTile::Obstacle => write!(f, "O")?,
                    MetaTile::Water => write!(f, "X")?,
                    MetaTile::Sprite(_) => write!(f, "S")?,
                    MetaTile::Warp { .. } => write!(f, "W")?,
                    MetaTile::Connection { .. } => write!(f, "C")?,
                    MetaTile::ConnectionWater(_) => write!(f, "~")?,
                    MetaTile::Jump(JumpDirection::South) => write!(f, "v")?,
                    MetaTile::Jump(JumpDirection::West)  => write!(f, "<")?,
                    MetaTile::Jump(JumpDirection::East)  => write!(f, ">")?,
                    MetaTile::Counter => write!(f, "=")?,
                    MetaTile::Switch { .. } => write!(f, "s")?,
                    MetaTile::CutTree => write!(f, "t")?,
                    // Never in `meta_tiles` either — a push, a cut and a Strength goal are
                    // actions on the ordinary floor beside the thing they are about, which is
                    // drawn as itself.
                    MetaTile::Cut { .. } | MetaTile::BoulderGoal { .. } => write!(f, "_")?,
                    MetaTile::Pc      => write!(f, "p")?,
                    MetaTile::Grass   => write!(f, "g")?,
                    // Never in `meta_tiles` — a fishing spot is an action on ordinary ground.
                    MetaTile::Fish { .. } => write!(f, "_")?,
                };
            }
            writeln!(f)?;
        }
        writeln!(f)
    }
}

/// Returns the position one step in `dir` from `pos`, or `None` if that would be out of bounds.
fn step_one(pos: Point8, dir: JoypadButton, width: usize, height: usize) -> Option<Point8> {
    match dir {
        JoypadButton::Up    => (pos.y > 0)
            .then(|| Point8 { x: pos.x, y: pos.y - 1 }),
        JoypadButton::Down  => (pos.y as usize + 1 < height)
            .then(|| Point8 { x: pos.x, y: pos.y + 1 }),
        JoypadButton::Left  => (pos.x > 0)
            .then(|| Point8 { x: pos.x - 1, y: pos.y }),
        JoypadButton::Right => (pos.x as usize + 1 < width)
            .then(|| Point8 { x: pos.x + 1, y: pos.y }),
        _ => None,
    }
}

/// Fixed PC-tile coordinates on `map` — hidden events the player faces (from below) and presses A
/// on.
pub fn pc_locations_for(map: Map) -> &'static [Point8] {
    /// The overwhelmingly common case: the PC on the back wall of a Pokémon Center, right of the
    /// healing counter.
    const CENTRE_PC: &[Point8] = &[Point8 { x: 13, y: 3 }];

    match map {
        Map::ViridianPokecenter | Map::PewterPokecenter | Map::CeruleanPokecenter
        | Map::LavenderPokecenter | Map::VermilionPokecenter | Map::CeladonPokecenter
        | Map::FuchsiaPokecenter | Map::CinnabarPokecenter | Map::MtMoonPokecenter
        | Map::RockTunnelPokecenter | Map::SaffronPokecenter
        | Map::CeladonHotel
        | Map::SafariZoneWestRestHouse | Map::SafariZoneEastRestHouse
        | Map::SafariZoneNorthRestHouse => CENTRE_PC,

        // Bill's cell-separator PC — used mid-SS-Ticket script (stand at (1,5) facing up + A).
        Map::BillsHouse => &[Point8 { x: 1, y: 4 }],
        // The player's own bedroom PC.
        Map::RedsHouse2F => &[Point8 { x: 0, y: 1 }],
        Map::CeladonMansion2F => &[Point8 { x: 0, y: 5 }],
        Map::IndigoPlateauLobby => &[Point8 { x: 15, y: 7 }],
        // The only map with two: the fossil room's lab machines both open the PC menu.
        Map::CinnabarLabFossilRoom => &[Point8 { x: 0, y: 4 }, Point8 { x: 2, y: 4 }],
        Map::SilphCo11F => &[Point8 { x: 10, y: 12 }],

        _ => &[],
    }
}

/// One hidden object the player can press A on, and how the cartridge insists on being
/// approached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HiddenObjectSite {
    /// The tile the player ends up facing, which is the one the ROM matches on. It is never
    /// walkable: a hidden object is drawn as the wall or the scenery it hides in.
    pub at: Point8,
    pub object: HiddenObject,
    /// The direction the player must be facing, where the routine behind the object checks.
    /// `None` means any side that can be reached will do.
    pub facing: Option<PlayerFacingDirection>,
}

impl HiddenObjectSite {
    const fn new(x: u8, y: u8, object: HiddenObject, facing: Option<PlayerFacingDirection>) -> Self {
        Self { at: Point8 { x, y }, object, facing }
    }
}

/// Every hidden object on `map` that a playthrough has to press, transcribed from pokered's
/// `data/events/hidden_events.asm` and `data/maps/objects/*.asm`.
pub fn hidden_objects_for(map: Map) -> &'static [HiddenObjectSite] {
    use HiddenObject::{Poster, Statue, TrashCan, VendingMachine};
    const UP: Option<PlayerFacingDirection> = Some(PlayerFacingDirection::Up);
    const ANY: Option<PlayerFacingDirection> = None;

    /// The bins, in the reading order the puzzle's own `wGymTrashCanIndex` numbers them.
    const VERMILION_BINS: &[HiddenObjectSite] = &[
        HiddenObjectSite::new(1,  7, TrashCan, ANY), HiddenObjectSite::new(1,  9, TrashCan, ANY),
        HiddenObjectSite::new(1, 11, TrashCan, ANY), HiddenObjectSite::new(3,  7, TrashCan, ANY),
        HiddenObjectSite::new(3,  9, TrashCan, ANY), HiddenObjectSite::new(3, 11, TrashCan, ANY),
        HiddenObjectSite::new(5,  7, TrashCan, ANY), HiddenObjectSite::new(5,  9, TrashCan, ANY),
        HiddenObjectSite::new(5, 11, TrashCan, ANY), HiddenObjectSite::new(7,  7, TrashCan, ANY),
        HiddenObjectSite::new(7,  9, TrashCan, ANY), HiddenObjectSite::new(7, 11, TrashCan, ANY),
        HiddenObjectSite::new(9,  7, TrashCan, ANY), HiddenObjectSite::new(9,  9, TrashCan, ANY),
        HiddenObjectSite::new(9, 11, TrashCan, ANY),
    ];
    const CELADON_DRINKS: &[HiddenObjectSite] = &[
        HiddenObjectSite::new(10, 1, VendingMachine, ANY),
        HiddenObjectSite::new(11, 1, VendingMachine, ANY),
        HiddenObjectSite::new(12, 2, VendingMachine, ANY),
    ];
    const GAME_CORNER_POSTER: &[HiddenObjectSite] = &[HiddenObjectSite::new(9, 4, Poster, ANY)];
    const MANSION_1F: &[HiddenObjectSite] = &[HiddenObjectSite::new(2, 5, Statue, UP)];
    const MANSION_2F: &[HiddenObjectSite] = &[HiddenObjectSite::new(2, 11, Statue, UP)];
    const MANSION_3F: &[HiddenObjectSite] = &[HiddenObjectSite::new(10, 5, Statue, UP)];
    const MANSION_B1F: &[HiddenObjectSite] = &[
        HiddenObjectSite::new(20, 3, Statue, UP),
        HiddenObjectSite::new(18, 25, Statue, UP),
    ];

    const BILLS_SEPARATOR: &[HiddenObjectSite] =
        &[HiddenObjectSite::new(1, 4, HiddenObject::CellSeparator, UP)];

    match map {
        Map::BillsHouse        => BILLS_SEPARATOR,
        Map::VermilionGym      => VERMILION_BINS,
        Map::CeladonMartRoof   => CELADON_DRINKS,
        Map::GameCorner        => GAME_CORNER_POSTER,
        Map::PokemonMansion1F  => MANSION_1F,
        Map::PokemonMansion2F  => MANSION_2F,
        Map::PokemonMansion3F  => MANSION_3F,
        Map::PokemonMansionB1F => MANSION_B1F,
        _ => &[],
    }
}

/// The floor panel on `map` and the floors its menu lists, in menu order — `None` for the 245
/// maps that are not a lift.
pub fn elevator_for(map: Map) -> Option<(Point8, &'static [Map])> {
    const ROCKET: &[Map] = &[Map::RocketHideoutB1F, Map::RocketHideoutB2F, Map::RocketHideoutB4F];
    const CELADON: &[Map] = &[
        Map::CeladonMart1F, Map::CeladonMart2F, Map::CeladonMart3F, Map::CeladonMart4F,
        Map::CeladonMart5F,
    ];
    const SILPH: &[Map] = &[
        Map::SilphCo1F, Map::SilphCo2F, Map::SilphCo3F, Map::SilphCo4F, Map::SilphCo5F,
        Map::SilphCo6F, Map::SilphCo7F, Map::SilphCo8F, Map::SilphCo9F, Map::SilphCo10F,
        Map::SilphCo11F,
    ];
    match map {
        // B3F is missing on purpose: the hideout's lift serves three floors and the stairs serve
        // the fourth, which is the cartridge's arrangement rather than an omission here.
        Map::RocketHideoutElevator => Some((Point8 { x: 1, y: 1 }, ROCKET)),
        Map::CeladonMartElevator   => Some((Point8 { x: 3, y: 0 }, CELADON)),
        Map::SilphCoElevator       => Some((Point8 { x: 3, y: 0 }, SILPH)),
        _ => None,
    }
}

/// The word `use_field_move`'s `direction` argument takes, so a refusal names a push in the
/// vocabulary the model would have to type to make it.
fn push_word(dir: JoypadButton) -> &'static str {
    match dir {
        JoypadButton::Up => "up", JoypadButton::Down => "down",
        JoypadButton::Left => "left", JoypadButton::Right => "right",
        _ => "that way",
    }
}

fn opposite_dir(dir: JoypadButton) -> JoypadButton {
    match dir {
        JoypadButton::Left  => JoypadButton::Right,
        JoypadButton::Right => JoypadButton::Left,
        JoypadButton::Up    => JoypadButton::Down,
        JoypadButton::Down  => JoypadButton::Up,
        other               => other,
    }
}
#[cfg(test)]
mod pc_location_tests {
    use super::*;

    /// Every Pokémon Center has a PC, and it is at the same place in all of them.
    #[test]
    fn every_pokemon_center_has_a_pc() {
        const CENTRES: &[Map] = &[
            Map::ViridianPokecenter, Map::PewterPokecenter, Map::CeruleanPokecenter,
            Map::LavenderPokecenter, Map::VermilionPokecenter, Map::CeladonPokecenter,
            Map::FuchsiaPokecenter, Map::CinnabarPokecenter, Map::MtMoonPokecenter,
            Map::RockTunnelPokecenter, Map::SaffronPokecenter,
        ];
        for &map in CENTRES {
            assert_eq!(pc_locations_for(map), &[Point8 { x: 13, y: 3 }], "no PC on {map}");
        }
    }

    /// The bins have to be the ones the puzzle itself numbers, and a transposed `(x, y)` is the
    /// failure this catches.
    #[test]
    fn the_gym_bins_are_the_ones_the_puzzle_numbers() {
        let bins = hidden_objects_for(Map::VermilionGym);
        assert_eq!(bins.len(), 15, "the gym has fifteen bins");
        for (index, site) in bins.iter().enumerate() {
            assert_eq!(site.object, HiddenObject::TrashCan);
            assert_eq!(
                site.at,
                crate::pokemon::trash_can_position(index as u8),
                "bin {index} is not where the puzzle's own index says",
            );
            assert!(site.facing.is_none(), "GymTrashScript checks no facing");
        }
    }

    /// The rest of the table, against the disassembly.
    #[test]
    fn hidden_objects_are_where_the_disassembly_says() {
        use PlayerFacingDirection::Up;
        let sites = |map| hidden_objects_for(map).iter()
            .map(|site| (site.at, site.object, site.facing)).collect::<Vec<_>>();

        assert_eq!(sites(Map::CeladonMartRoof), vec![
            (Point8 { x: 10, y: 1 }, HiddenObject::VendingMachine, None),
            (Point8 { x: 11, y: 1 }, HiddenObject::VendingMachine, None),
            (Point8 { x: 12, y: 2 }, HiddenObject::VendingMachine, None),
        ]);
        assert_eq!(sites(Map::GameCorner), vec![(Point8 { x: 9, y: 4 }, HiddenObject::Poster, None)]);
        assert_eq!(sites(Map::PokemonMansion1F),  vec![(Point8 { x: 2,  y: 5  }, HiddenObject::Statue, Some(Up))]);
        assert_eq!(sites(Map::PokemonMansion2F),  vec![(Point8 { x: 2,  y: 11 }, HiddenObject::Statue, Some(Up))]);
        assert_eq!(sites(Map::PokemonMansion3F),  vec![(Point8 { x: 10, y: 5  }, HiddenObject::Statue, Some(Up))]);
        assert_eq!(sites(Map::PokemonMansionB1F), vec![
            (Point8 { x: 20, y: 3  }, HiddenObject::Statue, Some(Up)),
            (Point8 { x: 18, y: 25 }, HiddenObject::Statue, Some(Up)),
        ]);
        // Bill's is the same tile `pc_locations_for` names, counted twice on purpose.
        assert_eq!(sites(Map::BillsHouse), vec![(Point8 { x: 1, y: 4 }, HiddenObject::CellSeparator, Some(Up))]);
        assert_eq!(pc_locations_for(Map::BillsHouse), &[Point8 { x: 1, y: 4 }]);
        // A Pokémon Centre has a PC and no hidden object, which is what keeps the two tables
        // apart.
        assert!(hidden_objects_for(Map::CeruleanPokecenter).is_empty());
        assert!(hidden_objects_for(Map::PalletTown).is_empty());
    }

    /// The three lifts, and the floors each one's own `*ElevatorWarpMaps` table lists — in order,
    /// because the index is the cursor row `DisplayElevatorFloorMenu` lands on.
    #[test]
    fn the_lifts_serve_the_floors_the_disassembly_lists() {
        assert_eq!(
            elevator_for(Map::RocketHideoutElevator).map(|(panel, floors)| (panel, floors.len())),
            Some((Point8 { x: 1, y: 1 }, 3)),
            "the hideout's lift serves B1F, B2F and B4F, and B3F is the stairs",
        );
        let (_, floors) = elevator_for(Map::RocketHideoutElevator).expect("a lift");
        assert_eq!(floors[2], Map::RocketHideoutB4F, "B4F is menu row 2 — Giovanni, so the Silph Scope");
        assert_eq!(elevator_for(Map::SilphCoElevator).map(|(panel, floors)| (panel, floors.len())),
                   Some((Point8 { x: 3, y: 0 }, 11)));
        assert_eq!(elevator_for(Map::CeladonMartElevator).map(|(panel, floors)| (panel, floors.len())),
                   Some((Point8 { x: 3, y: 0 }, 5)));
        assert!(elevator_for(Map::CeladonMart1F).is_none(), "a floor is not a lift");
    }

    /// The exceptions, which is the whole reason this is a table and not a constant.
    #[test]
    fn non_centre_pcs_are_where_the_disassembly_says() {
        assert_eq!(pc_locations_for(Map::BillsHouse),           &[Point8 { x: 1,  y: 4  }]);
        assert_eq!(pc_locations_for(Map::RedsHouse2F),          &[Point8 { x: 0,  y: 1  }]);
        assert_eq!(pc_locations_for(Map::CeladonMansion2F),     &[Point8 { x: 0,  y: 5  }]);
        assert_eq!(pc_locations_for(Map::IndigoPlateauLobby),   &[Point8 { x: 15, y: 7  }]);
        assert_eq!(pc_locations_for(Map::SilphCo11F),           &[Point8 { x: 10, y: 12 }]);
        assert_eq!(pc_locations_for(Map::CinnabarLabFossilRoom),
            &[Point8 { x: 0, y: 4 }, Point8 { x: 2, y: 4 }]);
        // The Safari *Center* rest house is the one of the four without a PC.
        assert!(pc_locations_for(Map::SafariZoneCenterRestHouse).is_empty());
        assert!(pc_locations_for(Map::PalletTown).is_empty());
    }
}

#[cfg(test)]
mod boulder_solver_tests {
    use super::*;
    use crate::pokemon::sprite::{Sprite, PictureId};

    /// Build a synthetic `MetaTileMap` from ASCII: `#`=wall, `.`=floor, `P`=player,
    /// `S`=switch(floor), `W`=an inter-map warp tile (walkable — the player may stand on it to
    /// push), digits `1..9`=boulders, `=`=a counter (for the reach-over-a-desk test below; the
    /// solver never meets one).
    fn from_ascii(rows: &[&str]) -> (MetaTileMap, Point8) {
        let h = rows.len();
        let w = rows[0].len();
        let mut meta = vec![MetaTile::Obstacle; w * h];
        let mut sprites = vec![];
        let mut player = Point8 { x: 0, y: 0 };
        let mut switch = Point8 { x: 0, y: 0 };
        for (y, row) in rows.iter().enumerate() {
            for (x, c) in row.chars().enumerate() {
                let p = Point8 { x: x as u8, y: y as u8 };
                let idx = x + y * w;
                match c {
                    '#' => {}
                    '.' => meta[idx] = MetaTile::Empty,
                    'W' => meta[idx] = MetaTile::Warp { to_map: Map::Route23, to_position: Point8 { x: 0, y: 0 } },
                    'w' => meta[idx] = MetaTile::Water,
                    '=' => meta[idx] = MetaTile::Counter,
                    'P' => { meta[idx] = MetaTile::Empty; player = p; }
                    'S' => { meta[idx] = MetaTile::Empty; switch = p; }
                    d if d.is_ascii_digit() => {
                        meta[idx] = MetaTile::Empty;
                        sprites.push(Sprite { index: d as u8, picture_id: PictureId::Monster,
                            position: p, on_screen: true, hidden: false,
                            facing: crate::pokemon::sprite::SpriteFacing::Down,
                            name: Box::leak(format!("Boulder {d}").into_boxed_str()) });
                    }
                    _ => panic!("bad char {c}"),
                }
            }
        }
        (MetaTileMap {
            walkable_cache: std::cell::OnceCell::new(),
            player_position: player, position_settled: true, surfing: false,
            player_direction: PlayerFacingDirection::Down,
            map: Map::VictoryRoad1F, width: w, height: h, meta_tiles: meta,
            raw_tile_ids: vec![0; w * h], tileset: crate::pokemon::map_header::TileSetId::Cavern,
            tile_pair_collisions: vec![],
            tile_pair_collisions_water: vec![], sprites,
            // Nobody is standing on anything in a hand-drawn fixture: the boulders this builder
            // paints are `MetaTile::Sprite` in `meta_tiles` and are meant to stay there.
            underfoot: vec![],
            warp_targets: HashSet::new(), connection_targets: HashSet::new(),
            spinners: HashMap::new(), script_cancelled_warps: vec![], standing_on_warp: true,
            can_surf: false, best_rod: None, can_cut: false,
            can_strength: false, bill_cell_separator: false,
            strength_switches: vec![switch], holes: vec![], no_surf_mount: HashSet::new(),
            has_grass_encounters: false,
            // A hand-built grid for the boulder solver; there is no ROM map behind it to draw.
            metadata: None,
        }, switch)
    }

    #[test]
    fn solves_trivial_one_push() {
        // Player at (1,2) pushes boulder (2,2) right onto the switch (3,2).
        let (map, switch) = from_ascii(&["#####", "#...#", "#P1S#", "#...#", "#####"]);
        let sol = map.solve_boulder_push(switch).expect("should solve");
        assert_eq!(sol, vec![(Point8 { x: 2, y: 2 }, JoypadButton::Right)]);
    }

    #[test]
    fn solves_two_push_around_corner() {
        // Boulder (2,2) → up to (2,1) [player below] → right to (3,1)=switch [player must go
        // AROUND to (1,1)].
        let (map, switch) = from_ascii(&["#####", "#..S#", "#.1.#", "#P..#", "#####"]);
        let sol = map.solve_boulder_push(switch).expect("should solve the around-corner push");
        assert_eq!(sol.last().unwrap().1, JoypadButton::Right);
        assert!(sol.len() >= 2, "needs at least two pushes, got {sol:?}");
    }

    #[test]
    fn solves_push_while_standing_on_warp() {
        // The VR1F crux in miniature: the switch (2,1) can only be reached by pushing the boulder
        // (2,2) UP, which requires the player to stand DIRECTLY BELOW it at (2,3) — and that tile
        // is a warp (like VR1F's entrance warp at (8,17)).
        let (map, switch) = from_ascii(&["#####", "#.S.#", "#.1.#", "#PW.#", "#####"]);
        let sol = map.solve_boulder_push(switch).expect("must solve by standing on the warp tile");
        assert_eq!(sol.last().unwrap(), &(Point8 { x: 2, y: 2 }, JoypadButton::Up));
    }

    /// A fishing row's route ends by *facing* the water: the last button is a turn, and no button
    /// before it steps onto water at all.
    #[test]
    fn a_fishing_rows_last_button_faces_the_water_rather_than_entering_it() {
        // One puddle with land all round it, the party able to Surf so the search is free to
        // cross, and the player approaching from a direction that is not the one it will end up
        // facing — which is what makes the turn a button of its own rather than the last walking
        // step.
        let (mut map, _) = from_ascii(&[
            "#######",
            "#....P#",
            "#.w.###",
            "#.....#",
            "#######",
        ]);
        map.can_surf = true;
        map.best_rod = Some(crate::pokemon::postgame::fishing::Rod::Super);
        map.map = Map::ViridianCity;
        map.tileset = crate::pokemon::map_header::TileSetId::Overworld;
        let water = Point8 { x: 2, y: 2 };

        let fish = map.actions().into_iter()
            .find(|a| matches!(a.tile, MetaTile::Fish { .. }))
            .expect("a rod, a water tileset and a reachable shore is a fishing row");

        assert_eq!(map.tile_at(fish.destination), MetaTile::Empty,
            "the row's destination is the shore square, never the water");

        let step = |p: Point8, b: JoypadButton| match b {
            JoypadButton::Up => Point8 { x: p.x, y: p.y - 1 },
            JoypadButton::Down => Point8 { x: p.x, y: p.y + 1 },
            JoypadButton::Left => Point8 { x: p.x - 1, y: p.y },
            JoypadButton::Right => Point8 { x: p.x + 1, y: p.y },
            other => panic!("a walk to a shore is directions only, got {other:?}"),
        };
        let (turn, walk) = fish.route.split_last().expect("a route with at least the turn on it");

        let mut pos = map.player_position;
        for &button in walk {
            pos = step(pos, button);
            assert_ne!(map.tile_at(pos), MetaTile::Water,
                "no button but the last may touch water: {:?}", fish.route);
        }
        assert_eq!(pos, fish.destination, "the walk ends on the shore square");
        assert_eq!(step(pos, *turn), water,
            "and the last button is the turn toward the water, not a step into it: {:?}", fish.route);
    }

    /// A nurse, a clerk and a receptionist are all talked to *over* something.
    #[test]
    fn an_interaction_reaches_over_a_counter() {
        // Player at (2,3), counter at (2,2), the person behind it at (2,1).
        let (mut map, _) = from_ascii(&["#####", "#.1.#", "#.=.#", "#.P.#", "#####"]);
        // `from_ascii` leaves a sprite's own cell walkable, because the boulder solver moves
        // sprites about and reads them from `sprites`.
        map.meta_tiles[2 + map.width] = MetaTile::Sprite("Boulder 1");

        map.player_direction = PlayerFacingDirection::Up;
        assert_eq!(
            map.interaction_in_front(),
            Some((Point8 { x: 2, y: 1 }, MetaTile::Sprite("Boulder 1"))),
            "the A press talks to the person behind the counter, so that is what was reached",
        );
        assert_eq!(
            map.tile_in_front(),
            Some((Point8 { x: 2, y: 2 }, MetaTile::Counter)),
            "the literal tile in front is unchanged: `cut` and friends are still asked about that one",
        );

        // Nobody behind it: the counter is what is being faced and there is nothing to look past.
        map.sprites.clear();
        map.meta_tiles[2 + map.width] = MetaTile::Empty;
        assert_eq!(map.interaction_in_front(), Some((Point8 { x: 2, y: 2 }, MetaTile::Counter)));

        map.player_direction = PlayerFacingDirection::Down;
        assert_eq!(
            map.interaction_in_front(),
            Some((Point8 { x: 2, y: 4 }, MetaTile::Obstacle)),
            "the hop happens only across a counter",
        );
    }

    #[test]
    fn reports_unsolvable() {
        // Boulder walled so it can only wobble left/right in a 1-wide slot, never reaching the
        // switch.
        let (map, switch) = from_ascii(&["#######", "#.....#", "#.###.#", "#.#1#.#", "#.#.#.#", "#..P.S#", "#######"]);
        // The boulder at (3,3) sits in a vertical dead-end; the switch (5,5) is unreachable for
        // it.
        assert!(map.solve_boulder_push(switch).is_none());
    }
}
