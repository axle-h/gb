use std::cmp::PartialEq;
use std::collections::{HashMap, HashSet};
use std::fmt::{Display, Formatter};
use poke_core::geometry::Point8;
use gb::joypad::JoypadButton;
use crate::pokemon::map::Map;
use crate::pokemon::actions::OverworldAction;
use crate::pokemon::map_metadata::{facing_button, CurrentMap, PlayerFacingDirection};
use crate::pokemon::tile::JumpDirection;
use crate::pokemon::sprite::Sprite;
use crate::pokemon::tile::{HiddenObject, MetaTile};

#[derive(Debug, Clone, Default)]
pub struct MetaTileMap {
    pub player_position: Point8,
    /// False while a map transition is in flight; nothing may draw a conclusion from
    /// [`Self::player_position`] while it is.
    pub position_settled: bool,
    /// The player is on the water rather than on foot (`wWalkBikeSurfState == 2`).
    pub surfing: bool,
    pub player_direction: PlayerFacingDirection,
    pub map: Map,
    pub width: usize,
    pub height: usize,
    pub meta_tiles: Vec<MetaTile>,
    /// Bottom-left raw tile id of each meta-tile, parallel to `meta_tiles`.
    pub raw_tile_ids: Vec<u8>,
    /// Kept for per-tileset ROM tables, such as the one [`Self::is_step_on_warp`] reads.
    pub tileset: crate::pokemon::map_header::TileSetId,
    /// Raw-tile-id pairs the player may not walk between (pokered `TilePairCollisionsLand`).
    pub tile_pair_collisions: Vec<(u8, u8)>,
    /// [`Self::walkable_bits`]'s answer, computed at most once per instance.
    walkable_cache: std::cell::OnceCell<Vec<u64>>,
    /// The pairs that apply when water is on either side of the move (`TilePairCollisionsWater`);
    /// in Seafoam they confine getting on and off the water to shore tiles.
    pub tile_pair_collisions_water: Vec<(u8, u8)>,
    pub sprites: Vec<Sprite>,
    /// Each person's square and the tile under them, read only by [`Self::row_blocked_by_people`].
    pub underfoot: Vec<(Point8, MetaTile)>,
    /// Distinct `(map, landing)` pairs behind warp tiles, keyed on the landing so two warps into
    /// different parts of one map are two rows.
    pub warp_targets: HashSet<(Map, Point8)>,
    pub connection_targets: HashSet<Map>,
    /// Spinner tile to the square its forced slide deposits the player on.
    pub spinners: HashMap<Point8, Point8>,
    /// `BIT_STANDING_ON_WARP`: the last completed step landed on a warp entry.
    pub standing_on_warp: bool,
    /// Squares whose warp the map's own script cancels, in padded coordinates.
    /// [`Self::warp_trigger`] answers `Impossible` for them, which keeps them out of `actions()`
    /// and `OverworldMovement` both.
    pub script_cancelled_warps: Vec<Point8>,
    /// Soul Badge, a Surf user, and not on Cycling Road, where `IsSurfingAllowed` refuses beside
    /// the sea. When set, the search crosses `Water`.
    pub can_surf: bool,
    /// The best rod in the bag, set by `game_state()` because the map builder has no bag access.
    pub best_rod: Option<crate::pokemon::postgame::fishing::Rod>,
    /// Cascade Badge and a Cut user, set by `game_state()` like [`Self::can_surf`].
    pub can_cut: bool,
    /// Rainbow Badge and a Strength user, set by `game_state()`.
    pub can_strength: bool,
    /// Is Bill waiting inside his own machine?
    pub bill_cell_separator: bool,
    /// Strength switch tiles from the ROM map scripts, so no policy hard-codes where to push.
    pub strength_switches: Vec<Point8>,
    /// Floor holes: the player and a pushed boulder both fall through. Routed as `MetaTile::Warp`
    /// (`apply_victory_road_holes`); this list is for discovery.
    pub holes: Vec<Point8>,
    /// Shore tiles Surf may not be mounted from ("The current is much too fast!").
    pub no_surf_mount: HashSet<Point8>,
    pub has_grass_encounters: bool,
    /// An indoor map outside the forest tileset rolls its grass rate on every floor tile
    /// (`TryDoWildEncounter`), so a cave floor is as good as grass.
    pub floor_encounters: bool,
    pub has_water_encounters: bool,
    /// The metadata these tiles came from, so the map picture needs no MMU read.
    pub metadata: Option<std::sync::Arc<crate::pokemon::map_metadata::MapMetadata>>,
}

/// Strength switch tiles per map, in raw coordinates (`VictoryRoad1F.asm` `.SwitchCoords`).
fn strength_switch_table(map: Map) -> &'static [(u8, u8)] {
    match map {
        Map::VictoryRoad1F => &[(17, 13)],
        Map::VictoryRoad2F => &[(1, 16), (9, 16)],
        Map::VictoryRoad3F => &[(3, 5)],
        _ => &[],
    }
}

/// Floor holes per map, in raw coordinates.
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

/// Shore tiles the game refuses to mount Surf from, in raw coordinates.
fn no_surf_mount_table(map: Map) -> &'static [(u8, u8)] {
    match map {
        Map::SeafoamIslandsB4F => &[(7, 11)],
        _ => &[],
    }
}

/// Spinner tile to slide destination, in raw coordinates, decoded from
/// `RocketHideout{2,3}ArrowTilePlayerMovement` read backwards.
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
        // `ViridianGymArrowTilePlayerMovement`: each arrow slides N tiles one way.
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

/// How the cartridge can be made to take the warp entry the player stands on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarpTrigger {
    /// A door or warp tile: arriving on it warps.
    StepOn,
    /// Warps only while this direction is held: the last step onto it, or a bump from on top.
    HoldDirection(JoypadButton),
    /// Nothing triggers it from this side.
    Impossible,
    /// Depends on a tile this model does not hold. Unsure is not no, so it is never dropped.
    Unknown,
}

/// One way off this map into an adjacent one: a run of touching edge tiles, not a tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Crossing {
    /// The tile to leave by, in action-id coordinates: the reachable one nearest the player if any.
    pub at: Point8,
    /// The landing on the far map, in raw coordinates; two runs into one map differ only here.
    pub to_position: Point8,
    /// Whether any tile of the run can be walked to from where the player is standing.
    pub reachable: bool,
    /// How many tiles the run holds.
    pub tiles: usize,
}

/// The price of a step from land onto water, in walking steps, so a search's `dist` is a price
/// rather than a step count.
const SURF_MOUNT_COST: u32 = 10;

/// The memo key for one `solve_boulder_push_tracking` question: exact rather than hashed, and
/// holding nothing that moves when an NPC steps.
#[derive(PartialEq, Eq, Hash)]
struct PlanKey {
    /// Stands for `raw_tile_ids` and the pair tables, which are fixed per map.
    map: Map,
    /// Two bits per tile, in reading order: may the player stand here, may a boulder land here.
    walkable: Vec<u64>,
    /// The visible boulders in the search's canonical order, `tracked` first.
    layout: Vec<Point8>,
    /// The player's component (its lowest tile), not their square.
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
            floor_encounters: map.grass_encounter_rate != 0
                && map.metadata.map as u8 >= Map::RedsHouse1F as u8
                && map.metadata.map_header.tileset != crate::pokemon::map_header::TileSetId::Forest,
            has_water_encounters: map.water_encounter_rate != 0,
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

    /// True if the bottom-left raw ids of `a` and `b` are a forbidden pair, checked symmetrically
    /// like `CheckForTilePairCollisions`.
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

    /// True if the warp on `point` fires on the step onto it (`CheckWarpsNoCollision`) rather than
    /// needing a direction held; position alone cannot tell the two apart.
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

    /// Every warp and connection tile the search reaches, with its position.
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

    /// Every tile the search reaches, including impassable tiles touching the walkable region.
    pub fn reachable_tiles(&self) -> std::collections::HashSet<Point8> {
        self.bfs_from_player().0.into_keys().collect()
    }

    /// A walk to the farthest reachable floor or water tile, never a warp or connection, to pace
    /// for encounters on a map with no grass.
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

    /// The raw tile `CheckForCollisionWhenPushingBoulder` refuses a boulder onto: stairs.
    const BOULDER_STAIRS_TILE: u8 = 0x15;

    /// The two refusals a push gets that a walk does not, for the stand and destination tiles.
    fn boulder_push_terrain_refusal(&self, stand: Point8, dest: Point8) -> Option<String> {
        // Two sentences: a model told about a step will look for a cliff at a staircase.
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

    /// Where the player can walk to shove a boulder, with each step's came-from. Not
    /// [`Self::bfs_from_player`]: a warp tile is standable here, and Victory Road 1F needs one.
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
                // Only a warp is asked what fires it.
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

    /// The walk to a push square under `push_search`'s rules, so the driver can make every walk the
    /// menu offers.
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

    /// Every one-tile shove the cartridge would carry out, as `(boulder, direction, stand square)`.
    pub fn boulder_pushes(&self) -> Vec<(Point8, JoypadButton, Point8)> {
        self.boulder_pushes_within(&self.push_search().0)
    }

    /// [`Self::boulder_pushes`] against a `push_search` already run, so `actions()` floods once.
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

    /// Why the cartridge would refuse to shove `boulder` one tile in `dir`, as a sentence the model
    /// can act on.
    pub fn boulder_push_refusal(&self, boulder: Point8, dir: JoypadButton) -> Option<String> {
        let name = self.sprites.iter()
            .find(|s| s.name.starts_with("Boulder") && !s.hidden && s.position == boulder)
            .map(|s| s.name);
        let Some(name) = name else {
            return Some(format!(
                "There is no boulder at ({}, {}). `read_map` gives the exact position of every one \
                 on this map.", boulder.x, boulder.y));
        };
        // One flood fill for all four directions.
        let reach = self.push_search().0;
        let refused = |why: String| {
            let ways: Vec<&str> = [(JoypadButton::Up, "up"), (JoypadButton::Down, "down"),
                                   (JoypadButton::Left, "left"), (JoypadButton::Right, "right")]
                .into_iter()
                .filter(|(d, _)| *d != dir && self.boulder_push_refusal_inner(boulder, *d, &reach).is_none())
                .map(|(_, w)| w).collect();
            let rest = match ways.as_slice() {
                // A stuck boulder is a reset, not a dead end, and the model is told both halves.
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

    /// [`Self::boulder_push_refusal`] without the prose, so the other directions can be asked.
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
            // A hole is a legal destination (Victory Road 3F, Seafoam), so a warp tile is allowed.
            MetaTile::Empty | MetaTile::Grass | MetaTile::Warp { .. } => {}
            MetaTile::Sprite(who) => return Some(format!("{who} is standing in the way at ({}, {})", dest.x, dest.y)),
            other => return Some(format!("({}, {}) is {other}", dest.x, dest.y)),
        }
        if let Some(refusal) = self.boulder_push_terrain_refusal(stand, dest) {
            return Some(refusal);
        }
        // Not a cartridge check: the stand tile must be walkable as well as reachable.
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

    /// Plan one-tile pushes landing some visible boulder on `switch`, as `(boulder before the push,
    /// direction)`; `None` if none can get there.
    pub fn solve_boulder_push(&self, switch: Point8) -> Option<Vec<(Point8, JoypadButton)>> {
        self.solve_boulder_push_tracking(None, switch)
    }

    /// [`Self::solve_boulder_push`] for one named boulder.
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
        // Only visible boulders are present and pushable.
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
        // The solver tracks boulders itself, so each starting square counts as floor although the
        // live tiles call it occupied.
        let initial: HashSet<Point8> = boulders.iter().copied().collect();
        let dirs = [(0i32, -1i32, JoypadButton::Up), (0, 1, JoypadButton::Down),
                    (-1, 0, JoypadButton::Left), (1, 0, JoypadButton::Right)];
        let inb = |x: i32, y: i32| x >= 0 && y >= 0 && (x as usize) < self.width && (y as usize) < self.height;
        let mv = |p: Point8, dx: i32, dy: i32| -> Option<Point8> {
            let (x, y) = (p.x as i32 + dx, p.y as i32 + dy);
            inb(x, y).then(|| Point8 { x: x as u8, y: y as u8 })
        };
        // A tile the player may stand on to push: floor, or a warp tile.
        let floor = |p: Point8| initial.contains(&p)
            || matches!(self.tile_at(p), MetaTile::Empty | MetaTile::Grass | MetaTile::Warp { .. });
        // A tile a boulder may land on: floor, or `switch` itself, which may be a hole.
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

        // An optimistic pre-filter, because an unsolvable pair is what costs the cap.
        let could_possibly_reach = |from: Point8| -> bool {
            let mut seen = HashSet::from([from]);
            let mut q = VecDeque::from([from]);
            while let Some(b) = q.pop_front() {
                if b == switch { return true; }
                for &(dx, dy, _) in &dirs {
                    let (Some(side), Some(dest)) = (mv(b, -dx, -dy), mv(b, dx, dy)) else { continue };
                    // Standing behind and landing in front depend on terrain alone, whatever the
                    // other boulders do.
                    if !floor(side) || !dest_floor(dest) { continue }
                    if self.boulder_push_terrain_refusal(side, dest).is_some() { continue }
                    if seen.insert(dest) { q.push_back(dest); }
                }
            }
            false
        };
        // Memoised, or `actions()` runs this search on every 20 ms tick.
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

        // A closure, so both exits land in the cache.
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
                    // The player must reach the square behind, the destination must be floor, and
                    // neither of `CheckForCollisionWhenPushingBoulder`'s extra refusals may apply.
                    if !r.contains(&side) || bs.contains(&dest) || !dest_floor(dest) { continue; }
                    // `side`, not `b`: the cartridge tests the player's tile.
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

    /// The shortest walk from the player to any reachable tile, not only a typed one.
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

    /// One search for many [`Self::route_to_face_within`] calls, rather than one per target.
    pub fn search_for_faces(&self) -> (HashMap<Point8, u32>, HashMap<Point8, (Point8, JoypadButton)>) {
        self.bfs_from_player()
    }

    /// The priced search: a mount costs [`SURF_MOUNT_COST`], so `dist` is not a step count.
    fn bfs_from_player(&self) -> (HashMap<Point8, u32>, HashMap<Point8, (Point8, JoypadButton)>) {
        let (dist, _, came_from) = self.search_from_player();
        (dist, came_from)
    }

    /// [`Self::bfs_from_player`] plus each tile's step count, for the one caller that means steps.
    fn search_from_player(&self)
        -> (HashMap<Point8, u32>, HashMap<Point8, u32>, HashMap<Point8, (Point8, JoypadButton)>) {
        use std::collections::{HashMap, HashSet, VecDeque};

        let mut dist: HashMap<Point8, u32> = HashMap::new();
        let mut steps: HashMap<Point8, u32> = HashMap::new();
        let mut came_from: HashMap<Point8, (Point8, JoypadButton)> = HashMap::new();
        let mut settled: HashSet<Point8> = HashSet::new();
        let mut buckets: Vec<VecDeque<Point8>> = vec![VecDeque::new()];

        // On a spinner, the search starts where the slide stops.
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

        // Buckets grow on demand.
        fn push(buckets: &mut Vec<VecDeque<Point8>>, price: u32, p: Point8) {
            let i = price as usize;
            if buckets.len() <= i { buckets.resize(i + 1, VecDeque::new()); }
            buckets[i].push_back(p);
        }

        let mut bucket = 0usize;
        while bucket < buckets.len() {
            while let Some(pos) = buckets[bucket].pop_front() {
                // A stale copy, found cheaper since or already expanded.
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

                    // A pad onto this same map (the Saffron Gym maze) lands at `to_position`.
                    if let MetaTile::Warp { to_map, to_position }
                        = self.meta_tiles[nb.x as usize + nb.y as usize * self.width]
                        && to_map == self.map
                    {
                        if (to_position.x as usize) < self.width && (to_position.y as usize) < self.height
                            && relax!(pos, to_position, dir, 1)
                        {
                            push(&mut buckets, dist[&to_position], to_position);
                        }
                        // The pad itself gets no `dist`, so no route ends on it.
                        continue;
                    }

                    if settled.contains(&nb) { continue; }

                    // A spinner slides the player to a fixed square.
                    if self.spinners.contains_key(&nb) {
                        let dest = self.resolve_spinner(nb);
                        if relax!(pos, dest, dir, 1) { push(&mut buckets, dist[&dest], dest); }
                        continue;
                    }

                    let tile = &self.meta_tiles[nb.x as usize + nb.y as usize * self.width];

                    if let MetaTile::Jump(jump_dir) = tile {
                        // A ledge is jumped, two tiles for one press, or it blocks.
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
                        // An elevation boundary (e.g. Cavern $20↔$05).
                        if self.pair_blocked(pos, nb) { continue; }
                        // Some shore tiles refuse a mount (Seafoam B4F's current).
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
                        // Warps and connections end a route: stepping on one fires the transition.
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

    /// True if the player can stand on this tile and walk on from it.
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

    /// A square of `kind` beside `at` that a step from `at` reaches, to pace between the two.
    pub fn pacing_neighbour(&self, at: Point8, kind: MetaTile) -> Option<Point8> {
        [JoypadButton::Up, JoypadButton::Down, JoypadButton::Left, JoypadButton::Right].into_iter()
            .filter_map(|dir| self.step(at, dir))
            .find(|&next| self.paces_on(next, kind) && !self.pair_blocked(at, next))
    }

    /// Whether standing on `p` rolls `kind`'s encounters. Water rolls only where the square's
    /// bottom-right tile is the plain water tile, so a shore square is water that never rolls.
    fn paces_on(&self, p: Point8, kind: MetaTile) -> bool {
        /// "in all tilesets with a water tile, this is its id" (`TryDoWildEncounter`).
        const WATER_TILE: u8 = 0x14;
        self.tile_at_checked(p) == Some(kind) && (kind != MetaTile::Water
            || self.metadata.as_ref()
                .and_then(|metadata| metadata.encounter_tile_ids.get(p.x as usize + p.y as usize * self.width))
                == Some(&WATER_TILE))
    }

    /// The nearest reachable square of `kind` with a [`Self::pacing_neighbour`].
    fn nearest_pacing_square(&self, kind: MetaTile, dist: &HashMap<Point8, u32>) -> Option<Point8> {
        dist.iter()
            .filter(|(p, _)| self.paces_on(**p, kind) && self.pacing_neighbour(**p, kind).is_some())
            .min_by_key(|(p, d)| (**d, p.y, p.x))
            .map(|(p, _)| *p)
    }

    /// Every row this map offers, from where the player is standing.
    pub fn actions(&self) -> Vec<OverworldAction> {
        if !self.position_settled {
            return vec![];
        }
        let (full_dist,     full_from)     = self.bfs_from_player();

        let reconstruct = |dest: Point8, came_from: &HashMap<Point8, (Point8, JoypadButton)>| -> Vec<JoypadButton> {
            let mut route = vec![];
            let mut pos = dest;
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
                // The cartridge's answer beats a guess from the map edge.
                WarpTrigger::HoldDirection(dir) => dir,
                _ => if dest.x == 0 { JoypadButton::Left }
                else if dest.x == (self.width - 1) as u8 { JoypadButton::Right }
                else if dest.y == 0 { JoypadButton::Up }
                else if dest.y == (self.height - 1) as u8 { JoypadButton::Down }
                else { *route.last().unwrap_or(&JoypadButton::Up) },
            };

            if route.is_empty() {
                match trigger {
                    // Surfing, or not on a warp the cartridge put you on, a held button does
                    // nothing; `OverworldMovement`'s border-warp arm holds the same rule.
                    WarpTrigger::HoldDirection(dir) if self.surfing || !self.standing_on_warp => {
                        route.push(opposite_dir(dir));
                        route.push(dir);
                    }
                    // One held button, not a step off and a step back.
                    WarpTrigger::HoldDirection(dir) => route.push(dir),
                    // A door warps on the step onto it: step off to a walkable neighbour and back.
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

        for sprite in self.sprites.iter().filter(|s| !s.hidden) {
            let sp = sprite.position;
            let direct: [(PlayerFacingDirection, Point8); 4] = [
                (PlayerFacingDirection::Down,  Point8 { x: sp.x,                   y: sp.y.saturating_sub(1) }),
                (PlayerFacingDirection::Up,    Point8 { x: sp.x,                   y: sp.y + 1               }),
                (PlayerFacingDirection::Right, Point8 { x: sp.x.saturating_sub(1), y: sp.y                   }),
                (PlayerFacingDirection::Left,  Point8 { x: sp.x + 1,               y: sp.y                   }),
            ];
            // Across a counter, the square one further away also talks to them.
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

            // Tall grass only where nothing else stands beside it, as in Viridian Forest's west side:
            // never in place of floor, so no row that already existed moves.
            let stand = |on: MetaTile| direct.iter().chain(counter_extra.iter())
                .filter(|(_, p)| {
                    (p.x as usize) < self.width && (p.y as usize) < self.height
                    && self.meta_tiles[p.x as usize + p.y as usize * self.width] == on
                    && best_dist_from(p).is_some()
                })
                .min_by_key(|(_, p)| best_dist_from(p).unwrap().0[p])
                .copied();
            let Some((face_dir, dest)) = stand(MetaTile::Empty).or_else(|| stand(MetaTile::Grass)) else { continue };

            let (_, came_from) = best_dist_from(&dest).unwrap();
            let mut route = reconstruct(dest, came_from);
            let face_button: JoypadButton = facing_button(face_dir);
            if route.is_empty() {
                if face_dir != self.player_direction { route.push(face_button); }
            } else if route.last() != Some(&face_button) {
                route.push(face_button);
            }
            route.push(JoypadButton::A);
            actions.push(OverworldAction { map: self.map, origin: self.player_position, destination: dest, tile: MetaTile::Sprite(sprite.name), route });
        }

        // One crossing per adjacent map per kind: one per edge perturbs the scripted run's timing,
        // and one per map hides a water crossing behind a nearer land one.
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

        if self.has_grass_encounters && let Some((_, dest)) = nearest(&|t| *t == MetaTile::Grass) {
            let route = reconstruct(dest, &full_from);
            actions.push(OverworldAction { map: self.map, origin: self.player_position, destination: dest, tile: MetaTile::Grass, route });
        } else if self.floor_encounters
            && let Some(dest) = self.nearest_pacing_square(MetaTile::Empty, &full_dist)
        {
            let route = reconstruct(dest, &full_from);
            actions.push(OverworldAction { map: self.map, origin: self.player_position, destination: dest,
                                           tile: MetaTile::Pace { water: false }, route });
        }
        if self.has_water_encounters && self.can_surf
            && let Some(dest) = self.nearest_pacing_square(MetaTile::Water, &full_dist)
        {
            let route = reconstruct(dest, &full_from);
            actions.push(OverworldAction { map: self.map, origin: self.player_position, destination: dest,
                                           tile: MetaTile::Pace { water: true }, route });
        }

        if let Some(rod) = self.best_rod
            && crate::pokemon::postgame::fishing::tileset_holds_water(self.tileset)
            && let Some(water) = crate::pokemon::postgame::fishing::nearest_castable_water(self)
        {
            // `Self::route_to_face_within`'s body inlined; the two must agree.
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
                let face_button: JoypadButton = facing_button(face_dir);
                if route.is_empty() {
                    if face_dir != self.player_direction { route.push(face_button); }
                } else if route.last() != Some(&face_button) {
                    route.push(face_button);
                }
                actions.push(OverworldAction { map: self.map, origin: self.player_position,
                    destination: dest, tile: MetaTile::Fish { rod }, route });
            }
        }

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

        for (index, site) in self.hidden_objects().iter().enumerate() {
            if site.object == HiddenObject::CellSeparator && !self.bill_cell_separator { continue }
            // Numbered over the whole table, so a bin's ordinal is `wGymTrashCanIndex` plus one.
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
            let face_button: JoypadButton = facing_button(face_dir);
            if route.is_empty() {
                if face_dir != self.player_direction { route.push(face_button); }
            } else if route.last() != Some(&face_button) {
                route.push(face_button);
            }
            actions.push(OverworldAction { map: self.map, origin: self.player_position,
                destination: dest, tile: MetaTile::Cut { at: tree }, route });
        }

        if self.can_strength && self.sprites.iter().any(|s| !s.hidden && s.name.starts_with("Boulder")) {
            // Walked with `push_search`: the square Victory Road's puzzle is pushed from is a warp
            // tile the routing search will not expand.
            let (reach, came) = self.push_search();

            // One row per target, naming the goal rather than the shove.
            let targets = self.strength_switches.iter().map(|at| (*at, false))
                .chain(self.holes.iter().map(|at| (*at, true)));
            for (at, hole) in targets {
                // A boulder already sits on it.
                if self.boulders().contains(&at) { continue }
                // Reachable by any boulder at all?
                if self.solve_boulder_push(at).is_none() { continue }
                // Then the nearest boulder that can do it.
                let mut candidates = self.boulders();
                candidates.sort_by_key(|b| (b.x as i32 - at.x as i32).abs() + (b.y as i32 - at.y as i32).abs());
                let Some((which, plan)) = candidates.into_iter()
                    .find_map(|b| self.solve_boulder_push_for(b, at).map(|plan| (b, plan)))
                    else { continue };
                let Some((boulder, push)) = plan.into_iter().next() else { continue };
                let Some(stand) = self.step(boulder, opposite_dir(push)) else { continue };
                if !reach.contains(&stand) { continue }
                // The walk is to the plan's first push; the driver re-plans from there.
                let mut route = reconstruct(stand, &came);
                if route.is_empty() {
                    let facing: JoypadButton = facing_button(self.player_direction);
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
        // A cancelling script runs after every tile test below, so it wins.
        if self.script_cancelled_warps.contains(&at) {
            return WarpTrigger::Impossible;
        }
        let Some(&here) = self.raw_tile_ids.get(at.x as usize + at.y as usize * self.width) else {
            return WarpTrigger::Impossible;
        };
        if self.tileset.warp_tile_ids().contains(&here)
            // ...or the pad-and-hole table `IsPlayerStandingOnWarpPadOrHole` reads.
            || self.tileset.warp_pad_and_hole_tile_ids().contains(&here)
        {
            return WarpTrigger::StepOn;
        }
        // `ExtraWarpCheck`'s dispatch order: SS Anne 3F takes function 1, four named maps function
        // 2, and only then does the tileset decide.
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
                Some(facing) => WarpTrigger::HoldDirection(facing_button(facing)),
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
                return WarpTrigger::HoldDirection(facing_button(facing));
            }
        }
        match looks_off_the_map {
            true => WarpTrigger::Unknown,
            false => WarpTrigger::Impossible,
        }
    }

    /// True when `row` is missing from [`Self::actions`] only because people stand in the way: with
    /// everyone but its subject put back `underfoot`, the row returns.
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
        // `walkable_bits` must be recomputed for the cleared floor.
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
        // Runs flood over 4-neighbours, so a wall splits one and a diagonal notch does not.
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
            // Named by its reachable tile nearest the player, else its first in reading order.
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

    /// The row crossing into `to_map` at raw `to_position`, if reachable. Kept out of `actions()`
    /// so `EnterMap { to_position }` can pick a landing without a row per edge.
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
            let facing: JoypadButton = facing_button(self.player_direction);
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

    /// Route to the nearest reachable `ConnectionWater` edge into `to_map`.
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

    /// What an A press here talks to: the tile in front, or the person behind a counter there.
    pub fn interaction_in_front(&self) -> Option<(Point8, MetaTile)> {
        let (at, tile) = self.tile_in_front()?;
        if tile != MetaTile::Counter { return Some((at, tile)); }
        let over = step_one(at, facing_button(self.player_direction), self.width, self.height);
        match over.map(|p| (p, self.tile_at(p))) {
            Some((p, sprite @ MetaTile::Sprite(_))) => Some((p, sprite)),
            _ => Some((at, tile)),
        }
    }

    /// Walk to an `Empty` tile beside `target` and turn to face it; empty if already there.
    pub fn route_to_face(&self, target: Point8) -> Option<Vec<JoypadButton>> {
        self.route_to_face_dir(target, None)
    }

    /// [`Self::route_to_face`] limited to the approach ending facing `required`, for switches such
    /// as the Mansion statues that check facing.
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
        let face_button: JoypadButton = facing_button(face_dir);
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
                    // Never in `meta_tiles`: an action on the floor beside a thing drawn as itself.
                    MetaTile::Cut { .. } | MetaTile::BoulderGoal { .. } | MetaTile::Pace { .. } => write!(f, "_")?,
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

/// Fixed PC tiles on `map`, approached from below.
pub fn pc_locations_for(map: Map) -> &'static [Point8] {
    /// A Pokémon Centre's PC, right of the counter.
    const CENTRE_PC: &[Point8] = &[Point8 { x: 13, y: 3 }];

    match map {
        Map::ViridianPokecenter | Map::PewterPokecenter | Map::CeruleanPokecenter
        | Map::LavenderPokecenter | Map::VermilionPokecenter | Map::CeladonPokecenter
        | Map::FuchsiaPokecenter | Map::CinnabarPokecenter | Map::MtMoonPokecenter
        | Map::RockTunnelPokecenter | Map::SaffronPokecenter
        | Map::CeladonHotel
        | Map::SafariZoneWestRestHouse | Map::SafariZoneEastRestHouse
        | Map::SafariZoneNorthRestHouse => CENTRE_PC,

        // Bill's cell separator, used during the SS Ticket script.
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

/// One hidden object the player can press A on, and how it must be approached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HiddenObjectSite {
    /// The tile the player faces, which the ROM matches on; never walkable.
    pub at: Point8,
    pub object: HiddenObject,
    /// The facing the object's routine checks, or `None` for any reachable side.
    pub facing: Option<PlayerFacingDirection>,
}

impl HiddenObjectSite {
    const fn new(x: u8, y: u8, object: HiddenObject, facing: Option<PlayerFacingDirection>) -> Self {
        Self { at: Point8 { x, y }, object, facing }
    }
}

/// The Cinnabar Gym's quiz machines and their questions, `hidden_events_for CINNABAR_GYM` and
/// `_CinnabarQuizQuestionsText1` to `6`. The right answers stay in the cartridge.
pub const CINNABAR_QUIZ_MACHINES: [(u8, u8, &str); 6] = [
    (15, 7, "CATERPIE evolves into BUTTERFREE?"),
    (10, 1, "There are 9 certified POKéMON LEAGUE BADGEs?"),
    (9, 7, "POLIWAG evolves 3 times?"),
    (9, 13, "Are thunder moves effective against ground element-type POKéMON?"),
    (1, 13, "POKéMON of the same kind and level are not identical?"),
    (1, 7, "TM28 contains TOMBSTONER?"),
];

/// Every hidden object a playthrough presses on `map`, from pokered's `hidden_events.asm`.
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

    /// Each machine twice, YES then NO, in `hidden_events_for CINNABAR_GYM` order.
    const CINNABAR_QUIZ: &[HiddenObjectSite] = &{
        let mut sites = [HiddenObjectSite::new(0, 0, HiddenObject::Quiz { yes: true }, UP); 12];
        let mut machine = 0;
        while machine < CINNABAR_QUIZ_MACHINES.len() {
            let (x, y, _) = CINNABAR_QUIZ_MACHINES[machine];
            sites[machine * 2] = HiddenObjectSite::new(x, y, HiddenObject::Quiz { yes: true }, UP);
            sites[machine * 2 + 1] = HiddenObjectSite::new(x, y, HiddenObject::Quiz { yes: false }, UP);
            machine += 1;
        }
        sites
    };

    match map {
        Map::BillsHouse        => BILLS_SEPARATOR,
        Map::CinnabarGym       => CINNABAR_QUIZ,
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

/// The floor panel on `map` and the floors its menu lists, in menu order; `None` if not a lift.
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
        // B3F is served by the stairs, not the lift.
        Map::RocketHideoutElevator => Some((Point8 { x: 1, y: 1 }, ROCKET)),
        Map::CeladonMartElevator   => Some((Point8 { x: 3, y: 0 }, CELADON)),
        Map::SilphCoElevator       => Some((Point8 { x: 3, y: 0 }, SILPH)),
        _ => None,
    }
}

/// The word `use_field_move`'s `direction` takes, so a refusal names a push as the model types it.
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

    /// The bins are the ones the puzzle numbers, catching a transposed `(x, y)`.
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
        // A Pokémon Centre has a PC and no hidden object.
        assert!(hidden_objects_for(Map::CeruleanPokecenter).is_empty());
        assert!(hidden_objects_for(Map::PalletTown).is_empty());
    }

    /// Each lift's floors match its `*ElevatorWarpMaps` table, in cursor order.
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

    /// The PCs outside a Pokémon Centre.
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

    /// A map from ASCII: `#` wall, `.` floor, `P` player, `S` switch, `W` warp (standable), `w`
    /// water, `=` counter, `1..9` boulders.
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
            // Hand-drawn boulders stay `MetaTile::Sprite`, with nothing underfoot.
            underfoot: vec![],
            warp_targets: HashSet::new(), connection_targets: HashSet::new(),
            spinners: HashMap::new(), script_cancelled_warps: vec![], standing_on_warp: true,
            can_surf: false, best_rod: None, can_cut: false,
            can_strength: false, bill_cell_separator: false,
            strength_switches: vec![switch], holes: vec![], no_surf_mount: HashSet::new(),
            has_grass_encounters: false, floor_encounters: false, has_water_encounters: false,
            // No ROM map behind it.
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
        // Up to (2,1), then right onto the switch after walking round to (1,1).
        let (map, switch) = from_ascii(&["#####", "#..S#", "#.1.#", "#P..#", "#####"]);
        let sol = map.solve_boulder_push(switch).expect("should solve the around-corner push");
        assert_eq!(sol.last().unwrap().1, JoypadButton::Right);
        assert!(sol.len() >= 2, "needs at least two pushes, got {sol:?}");
    }

    #[test]
    fn solves_push_while_standing_on_warp() {
        // The Victory Road 1F crux: the only push onto the switch is from a warp tile.
        let (map, switch) = from_ascii(&["#####", "#.S.#", "#.1.#", "#PW.#", "#####"]);
        let sol = map.solve_boulder_push(switch).expect("must solve by standing on the warp tile");
        assert_eq!(sol.last().unwrap(), &(Point8 { x: 2, y: 2 }, JoypadButton::Up));
    }

    /// A fishing row's route ends by facing the water, and no earlier button steps onto it.
    #[test]
    fn a_fishing_rows_last_button_faces_the_water_rather_than_entering_it() {
        // A puddle ringed by land, Surf allowed, approached from a side that makes the turn a
        // button of its own.
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

    /// A nurse, a clerk and a receptionist are all talked to over something.
    #[test]
    fn an_interaction_reaches_over_a_counter() {
        // Player at (2,3), counter at (2,2), the person behind it at (2,1).
        let (mut map, _) = from_ascii(&["#####", "#.1.#", "#.=.#", "#.P.#", "#####"]);
        // `from_ascii` leaves a sprite's cell walkable for the solver.
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
        // The boulder can only wobble in a one-wide slot.
        let (map, switch) = from_ascii(&["#######", "#.....#", "#.###.#", "#.#1#.#", "#.#.#.#", "#..P.S#", "#######"]);
        assert!(map.solve_boulder_push(switch).is_none());
    }
}
