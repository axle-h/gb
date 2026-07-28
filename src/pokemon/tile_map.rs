use std::cmp::PartialEq;
use std::collections::{HashMap, HashSet};
use std::fmt::{Display, Formatter};
use crate::geometry::Point8;
use crate::joypad::JoypadButton;
use crate::pokemon::map::Map;
use crate::pokemon::actions::OverworldAction;
use crate::pokemon::map_metadata::{CurrentMap, PlayerFacingDirection};
use crate::pokemon::tile::{JumpDirection, WarpEvent};
use crate::pokemon::sprite::Sprite;
use crate::pokemon::tile::MetaTile;

#[derive(Debug, Clone, Default)]
pub struct MetaTileMap {
    pub player_position: Point8,
    pub player_direction: PlayerFacingDirection,
    pub map: Map,
    pub width: usize,
    pub height: usize,
    pub meta_tiles: Vec<MetaTile>,
    /// Bottom-left raw tile ID of each meta-tile (parallel to `meta_tiles`). Used to evaluate
    /// `tile_pair_collisions` during BFS. `0xFF` for border/connection cells.
    pub raw_tile_ids: Vec<u8>,
    /// Unordered raw-tile-ID pairs the player may not walk between in this tileset (elevation
    /// boundaries from pokered `TilePairCollisionsLand`). Empty for most tilesets.
    pub tile_pair_collisions: Vec<(u8, u8)>,
    pub sprites: Vec<Sprite>,
    /// Unique `(destination_map, destination_position)` pairs reachable via warp tiles.
    /// Keyed on destination position so that two staircase/door warps that lead to
    /// *different* positions within the same destination map (e.g. Mt Moon B1F) each
    /// produce a separate `OverworldAction`.
    pub warp_targets: HashSet<(Map, Point8)>,
    pub connection_targets: HashSet<Map>,
    /// Arrow (spinner) tiles → the tile the forced slide deposits the player on. Stepping onto an
    /// arrow tile hands control to the game, which slides the player along a fixed path (decoded from
    /// the ROM `RocketHideout{2,3}ArrowTilePlayerMovement` tables). The BFS treats stepping onto an
    /// arrow as landing at its destination. Empty for maps without arrow tiles.
    pub spinners: HashMap<Point8, Point8>,
    /// True when the player can Surf (Soul Badge + a party mon that knows Surf). When set, the BFS
    /// treats `Water` tiles as passable so routes cross water; the agent mounts Surf at the land↔water
    /// boundary. Set by `game_state()` after construction (the map builder has no party access).
    pub can_surf: bool,
    /// Strength boulder-switch tiles on this map (invisible pressure plates, from the ROM map scripts):
    /// push a boulder onto one to open its barrier. Exposed so a policy (deterministic or LLM) can
    /// discover *where* to push without hardcoding coordinates. Empty for maps with no Strength puzzle.
    pub strength_switches: Vec<Point8>,
    /// Floor-hole tiles on this map (Victory Road 3F): the player can fall through one to the floor
    /// below, and pushing a boulder onto one drops it there (revealing a hidden boulder). Also modelled
    /// as `MetaTile::Warp` for routing (see `apply_victory_road_holes`); this list is for discovery.
    pub holes: Vec<Point8>,
}

/// Strength boulder-switch tiles per map (raw object/script coords, no connection offset), from the
/// pokered map scripts (e.g. `VictoryRoad1F.asm` `.SwitchCoords`). Pushing a boulder onto a switch runs
/// its `ReplaceTileBlock` (opens a barrier). Add other Strength maps (Seafoam, Rock Tunnel…) here.
fn strength_switch_table(map: Map) -> &'static [(u8, u8)] {
    match map {
        Map::VictoryRoad1F => &[(17, 13)],
        Map::VictoryRoad2F => &[(1, 16), (9, 16)],
        Map::VictoryRoad3F => &[(3, 5)],
        _ => &[],
    }
}

/// Floor-hole tiles per map (raw coords): a boulder pushed onto one falls to the floor below.
fn hole_table(map: Map) -> &'static [(u8, u8)] {
    match map {
        Map::VictoryRoad3F => &[(23, 15)],
        _ => &[],
    }
}

/// Arrow-tile → slide-destination tables for the spinner-floor maps (raw map coords), decoded from
/// the ROM movement RLE tables (`RocketHideout{2,3}ArrowTilePlayerMovement`, read backwards;
/// PAD_DOWN=+y, UP=−y, LEFT=−x, RIGHT=+x). Interior maps have no connection border, so these raw
/// coords need no west/north offset.
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
        // Viridian Gym (Giovanni / Earth Badge), decoded from `ViridianGymArrowTilePlayerMovement`
        // (all single-segment `db PAD_DIR, N`): each arrow at (x,y) slides N tiles → (tx,ty).
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


impl MetaTileMap {
    pub fn new(map: &CurrentMap) -> Self {
        let dimensions = map.metadata.dimensions();
        let width  = dimensions.full_width();
        let height = dimensions.full_height();
        // Clamp to valid tile coordinates. During map transitions wXCoord/wYCoord can
        // briefly hold values outside the new map's bounds; adding connection-strip
        // offsets can make them worse. Clamping prevents out-of-bounds tile accesses.
        let px = (map.player_position.x as usize + dimensions.west_extra).min(width.saturating_sub(1)) as u8;
        let py = (map.player_position.y as usize + dimensions.north_extra).min(height.saturating_sub(1)) as u8;
        let meta_tiles = map.meta_tiles();
        Self {
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
            warp_targets: meta_tiles.iter()
                .filter_map(|t| if let MetaTile::Warp { to_map, to_position } = t { Some((*to_map, *to_position)) } else { None })
                .collect(),
            connection_targets: meta_tiles.iter()
                .filter_map(|t| match t {
                    MetaTile::Connection { to_map, .. } => Some(*to_map),
                    // Water connections (a surfable map edge) are crossings too — surface them so
                    // `actions()` produces a route the agent can surf across to the connected map.
                    MetaTile::ConnectionWater(to_map) => Some(*to_map),
                    _ => None,
                })
                .collect(),
            raw_tile_ids: map.metadata.raw_tile_ids.clone(),
            tile_pair_collisions: map.metadata.tile_pair_collisions.clone(),
            spinners: spinner_table(map.metadata.map).iter().map(|&(x, y, tx, ty)| {
                let off = |px: u8, py: u8| Point8 {
                    x: px + dimensions.west_extra as u8,
                    y: py + dimensions.north_extra as u8,
                };
                (off(x, y), off(tx, ty))
            }).collect(),
            meta_tiles,
            can_surf: false,
            strength_switches: strength_switch_table(map.metadata.map).iter()
                .map(|&(x, y)| Point8 { x: x + dimensions.west_extra as u8, y: y + dimensions.north_extra as u8 })
                .collect(),
            holes: hole_table(map.metadata.map).iter()
                .map(|&(x, y)| Point8 { x: x + dimensions.west_extra as u8, y: y + dimensions.north_extra as u8 })
                .collect(),
        }
    }

    /// A direction to an `Empty` (freely walkable, not pair-blocked) neighbour of `pos`, if any.
    /// Used to step off a warp tile the player is standing on so it can be re-triggered by
    /// stepping back on.
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
    /// raw tile IDs form a forbidden pair in this tileset (pokered `TilePairCollisionsLand`).
    /// The check is symmetric, matching `CheckForTilePairCollisions`.
    fn pair_blocked(&self, a: Point8, b: Point8) -> bool {
        if self.tile_pair_collisions.is_empty() { return false; }
        let ta = self.raw_tile_ids[a.x as usize + a.y as usize * self.width];
        let tb = self.raw_tile_ids[b.x as usize + b.y as usize * self.width];
        self.tile_pair_collisions.iter().any(|&(t1, t2)| {
            (ta == t1 && tb == t2) || (ta == t2 && tb == t1)
        })
    }

    pub fn tile_at(&self, point: Point8) -> MetaTile {
        self.meta_tiles[point.x as usize + point.y as usize * self.width]
    }

    /// Bounds-checked `tile_at` — `None` if `point` is off the map.
    pub fn tile_at_checked(&self, point: Point8) -> Option<MetaTile> {
        if (point.x as usize) < self.width && (point.y as usize) < self.height {
            Some(self.meta_tiles[point.x as usize + point.y as usize * self.width])
        } else {
            None
        }
    }

    /// Follow the arrow-tile chain from `pos` to the tile the forced slide finally rests on. Returns
    /// `pos` unchanged if it isn't an arrow tile. Bounded against pathological cycles.
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

    /// Returns every warp and connection tile that is reachable by BFS from the player
    /// position, together with its expanded-coordinate position in the map.
    ///
    /// Unlike [`actions`], this does **not** deduplicate by destination map — multiple
    /// warp tiles leading to different entry points of the same destination map are all
    /// returned.  This is required by the world-graph builder, which must discover every
    /// reachable (source_tile, destination) pair so it does not miss cave sections that
    /// are only accessible via "non-nearest" warps.
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

    /// The set of tiles reachable from the player (debug/diagnostic aid for maze mapping).
    pub fn reachable_tiles(&self) -> std::collections::HashSet<Point8> {
        self.bfs_from_player().0.into_keys().collect()
    }

    /// A wander action to the farthest reachable WALKABLE tile: walking (or Surfing) there triggers a
    /// per-step encounter on a cave/water map that has no grass and no reachable cave object to pace toward
    /// (e.g. entering Seafoam in a pocket away from the boulders). Only Empty/Grass/Water destinations are
    /// considered — never a Warp/Connection tile (stepping onto one would leave the map). `None` if the only
    /// reachable tile is the player's own.
    pub fn wander_action(&self) -> Option<crate::pokemon::actions::OverworldAction> {
        let (dist, _) = self.bfs_from_player();
        let dest = dist.iter()
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

    /// Single-boulder Sokoban: plan a sequence of one-tile pushes that lands a boulder on `switch`.
    /// Each entry is `(boulder_position_before_that_push, push_direction)`; returns `None` if no boulder
    /// can reach the switch. Boulders are the sprites named "Boulder …"; the boulder being pushed treats
    /// the others (and walls/water/warps) as fixed obstacles. After a push the player ends on the
    /// boulder's old tile, so its reachable region is recomputed from there each step.
    pub fn solve_boulder_push(&self, switch: Point8) -> Option<Vec<(Point8, JoypadButton)>> {
        use std::collections::{HashMap, HashSet, VecDeque};
        // Only *visible* boulders are physically present and pushable. A hidden boulder (e.g. Victory
        // Road 2F's boulder that stays hidden until a 3F boulder falls through a hole onto it) must be
        // ignored, or the solver plans pushes of a phantom sprite that can never actually move.
        let boulders: Vec<Point8> = self.sprites.iter()
            .filter(|s| s.name.starts_with("Boulder") && !s.hidden)
            .map(|s| s.position).collect();
        let all: HashSet<Point8> = boulders.iter().copied().collect();
        let dirs = [(0i32, -1i32, JoypadButton::Up), (0, 1, JoypadButton::Down),
                    (-1, 0, JoypadButton::Left), (1, 0, JoypadButton::Right)];
        let inb = |x: i32, y: i32| x >= 0 && y >= 0 && (x as usize) < self.width && (y as usize) < self.height;
        let mv = |p: Point8, dx: i32, dy: i32| -> Option<Point8> {
            let (x, y) = (p.x as i32 + dx, p.y as i32 + dy);
            inb(x, y).then(|| Point8 { x: x as u8, y: y as u8 })
        };
        for &start in &boulders {
            let others: HashSet<Point8> = all.iter().copied().filter(|&b| b != start).collect();
            // A tile the player may STAND ON to push a boulder: ordinary floor, and also inter-map
            // warp tiles.  The player can legitimately stand on a coordinate-warp (e.g. Victory Road
            // 1F's entrance warps at (8,17)/(9,17), plain cave floor $21) and push a boulder past it —
            // the warp only fires when moving *onto* it toward the warp, not when pushing up into a
            // boulder (VR1F is Cavern, so `ExtraWarpCheck` = `IsWarpTileInFrontOfPlayer`, and the tile
            // in front when pushing is not a warp tile).  Excluding warps here was the bug that made
            // VR1F look unsolvable.
            // `self.tile_at` reports the *live* boulder sprites as occupied, but the solver simulates
            // boulders moving — so the tile UNDER any boulder must count as floor (the solver tracks
            // boulder occupancy itself via `active`/`others`). Without this, a boulder's own starting
            // tile stays a phantom wall after it has (in simulation) been pushed away.
            let under_boulder = |p: Point8| all.contains(&p);
            let player_stand = |p: Point8, active: Point8| p != active && !others.contains(&p)
                && (under_boulder(p) || matches!(self.tile_at(p), MetaTile::Empty | MetaTile::Grass | MetaTile::Warp { .. }));
            // A tile a BOULDER may be pushed onto: ordinary floor (or a tile vacated by a boulder), or
            // the explicit `switch` target itself — this lets the caller aim a boulder at a hole tile
            // (a `MetaTile::Warp`) to drop it to the floor below (Victory Road 3F), which normal floor
            // rules would reject. Any other warp/ladder is off-limits.
            let boulder_dest = |p: Point8, active: Point8| p != active && !others.contains(&p)
                && (p == switch || under_boulder(p) || matches!(self.tile_at(p), MetaTile::Empty | MetaTile::Grass));
            // Tiles the player can reach from `from`, with the active boulder + others as walls.
            // Respects tile-pair collisions (cave "cliffs") exactly like real player movement — without
            // this, vacating a boulder could wrongly appear to open a barrier the player can't cross.
            let reach = |active: Point8, from: Point8| -> HashSet<Point8> {
                let mut seen = HashSet::from([from]);
                let mut q = VecDeque::from([from]);
                while let Some(p) = q.pop_front() {
                    for &(dx, dy, _) in &dirs {
                        if let Some(n) = mv(p, dx, dy) {
                            if player_stand(n, active) && !self.pair_blocked(p, n) && seen.insert(n) {
                                q.push_back(n);
                            }
                        }
                    }
                }
                seen
            };
            // Complete single-boulder Sokoban: the state is (boulder position, player's connected
            // component) so the same boulder tile is revisited when the player can approach from a
            // different side. The component is represented by its lexicographically-smallest floor tile.
            let norm = |set: &HashSet<Point8>| -> Point8 {
                *set.iter().min_by_key(|p| (p.y, p.x)).unwrap()
            };
            // state key = (boulder, player_component_rep); value = (prev_state, push_dir, push_from_boulder)
            let mut came: HashMap<(Point8, Point8), ((Point8, Point8), JoypadButton)> = HashMap::new();
            let start_rep = norm(&reach(start, self.player_position));
            let mut visited: HashSet<(Point8, Point8)> = HashSet::from([(start, start_rep)]);
            let mut q = VecDeque::from([(start, self.player_position)]);
            let mut boulder_cells: HashSet<Point8> = HashSet::from([start]);
            while let Some((b, player_from)) = q.pop_front() {
                let r = reach(b, player_from);
                let brep = norm(&r);
                if b == switch {
                    if std::env::var("BOULDER_DEBUG").is_ok() { eprintln!("  boulder {start} CAN reach switch {switch}"); }
                    let mut pushes = vec![];
                    let mut state = (b, brep);
                    while let Some(&(prev_state, dir)) = came.get(&state) {
                        pushes.push((prev_state.0, dir)); // push the boulder from its previous position
                        state = prev_state;
                    }
                    pushes.reverse();
                    return Some(pushes);
                }
                for &(dx, dy, dir) in &dirs {
                    let (Some(side), Some(dest)) = (mv(b, -dx, -dy), mv(b, dx, dy)) else { continue };
                    // The player must be able to reach the tile behind the boulder, the destination must
                    // be plain floor, and there must be no elevation/tile-pair cliff between the boulder
                    // and its destination (pokered `CheckForCollisionWhenPushingBoulder`).
                    if r.contains(&side) && boulder_dest(dest, b) && !self.pair_blocked(b, dest) {
                        // After the push the player stands on `b`; recompute its component.
                        let dest_rep = norm(&reach(dest, b));
                        if visited.insert((dest, dest_rep)) {
                            came.insert((dest, dest_rep), ((b, brep), dir));
                            boulder_cells.insert(dest);
                            q.push_back((dest, b));
                        }
                    }
                }
            }
            if std::env::var("BOULDER_DEBUG").is_ok() {
                let mut cells: Vec<_> = boulder_cells.iter().copied().collect();
                cells.sort_by_key(|p| (p.y, p.x));
                eprintln!("  boulder {start}: reached {} cells: {:?}", cells.len(),
                    cells.iter().map(|p| (p.x, p.y)).collect::<Vec<_>>());
            }
        }
        None
    }

    /// The shortest walking route (button sequence) from the player to an arbitrary reachable tile,
    /// or `None` if unreachable. Used to position the player next to a boulder before a Strength push
    /// (the standard `actions()` routes only target *typed* tiles, not arbitrary floor positions).
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

    /// BFS from `player_position` outward.
    ///
    /// Returns `(dist, came_from)` where `dist[p]` is the minimum step-count to reach `p`
    /// and `came_from[p]` is the `(previous_position, direction)` on the shortest path.
    fn bfs_from_player(&self) -> (HashMap<Point8, u32>, HashMap<Point8, (Point8, JoypadButton)>) {
        use std::collections::{HashMap, VecDeque};

        let mut dist: HashMap<Point8, u32> = HashMap::new();
        let mut came_from: HashMap<Point8, (Point8, JoypadButton)> = HashMap::new();
        let mut queue = VecDeque::new();

        // If the player is standing on an arrow tile (mid-slide), the forced movement will carry them
        // to its rest destination — start the search from there.
        let start = self.resolve_spinner(self.player_position);
        dist.insert(start, 0);
        queue.push_back(start);

        while let Some(pos) = queue.pop_front() {
            let d = dist[&pos];
            let neighbors = [
                (JoypadButton::Up,    Point8 { x: pos.x,                    y: pos.y.wrapping_sub(1) }),
                (JoypadButton::Down,  Point8 { x: pos.x,                    y: pos.y.wrapping_add(1) }),
                (JoypadButton::Left,  Point8 { x: pos.x.wrapping_sub(1),    y: pos.y                 }),
                (JoypadButton::Right, Point8 { x: pos.x.wrapping_add(1),    y: pos.y                 }),
            ];
            for (dir, nb) in neighbors {
                if nb.x as usize >= self.width || nb.y as usize >= self.height { continue; }
                if dist.contains_key(&nb) { continue; }

                // Arrow (spinner) tile: stepping onto `nb` hands control to the game, which slides the
                // player to a fixed destination. Record an edge from `pos` (press `dir`) → that
                // destination; the player never stops on the arrow itself.
                if self.spinners.contains_key(&nb) {
                    let dest = self.resolve_spinner(nb);
                    if !dist.contains_key(&dest) {
                        dist.insert(dest, d + 1);
                        came_from.insert(dest, (pos, dir));
                        queue.push_back(dest);
                    }
                    continue;
                }

                let tile = &self.meta_tiles[nb.x as usize + nb.y as usize * self.width];

                // Intra-map teleporter (the Saffron Gym warp maze): stepping onto `nb` warps the
                // player to `to_position` on *this same map*. Like a spinner, the player never stops
                // on the pad — record an edge from `pos` (press `dir`) → the landing tile and continue
                // the search from there, so routes cross the maze automatically. Routes are recomputed
                // each tick, so after the warp the follower simply re-plans from the new room. (Regular
                // inter-map warps stay terminal — handled in the `else` branch below.)
                if let MetaTile::Warp { to_map, to_position } = tile {
                    if *to_map == self.map {
                        let dest = *to_position;
                        if (dest.x as usize) < self.width
                            && (dest.y as usize) < self.height
                            && !dist.contains_key(&dest)
                        {
                            dist.insert(dest, d + 1);
                            came_from.insert(dest, (pos, dir));
                            queue.push_back(dest);
                        }
                        continue;
                    }
                }

                if let MetaTile::Jump(jump_dir) = tile {
                    // The player never stands on a Jump tile — they either jump over it
                    // (one button press, two tiles of movement) or are blocked.
                    // Jump tiles are never added to `dist`; only the landing position is.
                    let can_jump = matches!((dir, jump_dir),
                        (JoypadButton::Down,  JumpDirection::South) |
                        (JoypadButton::Left,  JumpDirection::West)  |
                        (JoypadButton::Right, JumpDirection::East)
                    );
                    if can_jump {
                        if let Some(landing) = step_one(nb, dir, self.width, self.height) {
                            let landing_tile = &self.meta_tiles[landing.x as usize + landing.y as usize * self.width];
                            // Only record the landing if the player can actually stand on it. A
                            // blocked landing means the ledge is not traversable from here — recording
                            // it anyway would both invent a phantom route step (the agent presses the
                            // jump direction into an immovable ledge forever) and mark the tile as
                            // visited, hiding any genuine path that reaches it another way.
                            let landing_blocked = matches!(landing_tile,
                                MetaTile::Obstacle | MetaTile::Sprite(_) | MetaTile::Water |
                                MetaTile::ConnectionWater(_) | MetaTile::Jump(_)
                            );
                            if !landing_blocked && !dist.contains_key(&landing) {
                                dist.insert(landing, d + 1);
                                came_from.insert(landing, (pos, dir));
                                queue.push_back(landing);
                            }
                        }
                    }
                } else {
                    // Elevation boundary: the player cannot step between certain tile pairs
                    // even though both are passable (e.g. Cavern $20↔$05). Skip this edge so
                    // `nb` may still be reached from a non-blocked neighbour.
                    if self.pair_blocked(pos, nb) { continue; }
                    dist.insert(nb, d + 1);
                    came_from.insert(nb, (pos, dir));
                    // Warp and Connection tiles are terminal: the player can reach one but cannot
                    // walk *through* it, because stepping onto it fires the transition. Not
                    // queueing them keeps routes to a specific warp from crossing (and triggering)
                    // a different warp/connection en route.
                    //
                    // Water is normally terminal too — but when the player can Surf, plain `Water`
                    // becomes a pass-through node so routes can cross it (the agent mounts Surf at the
                    // land→water boundary). `ConnectionWater` stays terminal: stepping onto it while
                    // surfing crosses to the connected map (a crossing target, like `Connection`).
                    let surfable_water = self.can_surf && matches!(tile, MetaTile::Water);
                    if surfable_water || !matches!(tile,
                        MetaTile::Obstacle | MetaTile::Sprite(_) | MetaTile::Water
                        | MetaTile::ConnectionWater(_) | MetaTile::Counter | MetaTile::CutTree
                        | MetaTile::Warp { .. } | MetaTile::Connection { .. })
                    {
                        queue.push_back(nb);
                    }
                }
            }
        }
        (dist, came_from)
    }

    /// Fixed PC-tile coordinates on this map (hidden objects the player faces + A to use). These
    /// are not derivable from the tileset, so they are hard-coded from pokered
    /// `data/events/hidden_objects.asm`. `actions()` emits a face-and-A route to each.
    fn pc_locations(&self) -> &'static [Point8] {
        match self.map {
            // Bill's cell-separator PC — used mid-SS-Ticket script (stand at (1,5) facing up + A).
            Map::BillsHouse => &[Point8 { x: 1, y: 4 }],
            _ => &[],
        }
    }

    pub fn actions(&self) -> Vec<OverworldAction> {
        let (full_dist,     full_from)     = self.bfs_from_player();

        // Reconstruct the step sequence from the given came_from back-pointers.
        let reconstruct = |dest: Point8, came_from: &HashMap<Point8, (Point8, JoypadButton)>| -> Vec<JoypadButton> {
            let mut route = vec![];
            let mut pos = dest;
            // Walk back to the BFS root (the node with no predecessor). The root is normally the
            // player's position, but when the player is mid-slide on an arrow tile the BFS is rooted
            // at the slide's rest destination instead — so stop on the first node without a
            // `came_from` entry rather than testing against `player_position`.
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
            let Some((tile, dest)) = nearest(&|t| matches!(t, MetaTile::Warp { to_map, to_position } if to_map == warp_to_map && to_position == warp_to_pos)) else { continue };
            let (_, came_from) = best_dist_from(&dest).unwrap();
            let mut route = reconstruct(dest, came_from);

            let enter_dir = if dest.x == 0 { JoypadButton::Left }
            else if dest.x == (self.width - 1) as u8 { JoypadButton::Right }
            else if dest.y == 0 { JoypadButton::Up }
            else if dest.y == (self.height - 1) as u8 { JoypadButton::Down }
            else { *route.last().unwrap_or(&JoypadButton::Up) };

            if route.is_empty() {
                // Already standing on the warp tile: a warp fires on the step ONTO it, not while
                // standing still, so step off to a genuinely walkable neighbour then step back on.
                // Use a real Empty neighbour (defaulting Down can walk into a wall and jam).
                let step_off = self.walkable_neighbor_dir(dest).unwrap_or_else(|| opposite_dir(enter_dir));
                route.push(step_off);
                route.push(opposite_dir(step_off));
            } else {
                route.push(enter_dir);
            }
            actions.push(OverworldAction { map: self.map, origin: self.player_position, destination: dest, tile, route });
        }


        // 2. Routes to sprites (route to an adjacent empty tile, then face the sprite)
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
            // The player stands there, faces the counter, presses A — pokered then looks
            // through the counter to interact with the sprite behind it.
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

        // 3. Routes to map connections (nearest reachable connection tile per adjacent map).
        //    A *specific* landing (e.g. to avoid a dead-end pocket) is requested via
        //    `connection_action(to_map, to_position)`, kept out of this hot path so the common
        //    nearest-crossing behaviour — and the whole-game run's timing — is unchanged.
        for to_map in &self.connection_targets {
            let Some((tile, dest)) = nearest(&|t| match t {
                MetaTile::Connection { to_map: m, .. } => m == to_map,
                MetaTile::ConnectionWater(m) => m == to_map,
                _ => false,
            }) else { continue };
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

        // 4. Walk-in-grass (nearest reachable grass tile).
        if let Some((_, dest)) = nearest(&|t| *t == MetaTile::Grass) {
            let route = reconstruct(dest, &full_from);
            actions.push(OverworldAction { map: self.map, origin: self.player_position, destination: dest, tile: MetaTile::Grass, route });
        }

        // 5. PC tiles (hidden-object interactables): route to a walkable tile adjacent to the PC,
        //    face it, press A. Mirrors the sprite-interaction routing (a PC is not a sprite, so it
        //    is keyed by fixed coordinate rather than found in the sprite list).
        for &pc in self.pc_locations() {
            let adj: [(PlayerFacingDirection, Point8); 4] = [
                (PlayerFacingDirection::Down,  Point8 { x: pc.x,                   y: pc.y.saturating_sub(1) }),
                (PlayerFacingDirection::Up,    Point8 { x: pc.x,                   y: pc.y + 1               }),
                (PlayerFacingDirection::Right, Point8 { x: pc.x.saturating_sub(1), y: pc.y                   }),
                (PlayerFacingDirection::Left,  Point8 { x: pc.x + 1,               y: pc.y                   }),
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
            route.push(JoypadButton::A);
            actions.push(OverworldAction { map: self.map, origin: self.player_position, destination: dest, tile: MetaTile::Pc, route });
        }

        // 6. Cut trees: route to a walkable tile adjacent to a CutTree and face it (no A — the cut is
        //    triggered via the field-move menu once facing). One action per reachable-adjacent tree.
        let cut_trees: Vec<Point8> = self.meta_tiles.iter().enumerate()
            .filter(|(_, t)| **t == MetaTile::CutTree)
            .map(|(i, _)| Point8 { x: (i % self.width) as u8, y: (i / self.width) as u8 })
            .collect();
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
            actions.push(OverworldAction { map: self.map, origin: self.player_position, destination: dest, tile: MetaTile::CutTree, route });
        }

        actions.sort();
        actions
    }

    /// Build the action that crosses a connection to `to_map` landing at raw `to_position`, if that
    /// specific connection tile is reachable. Kept out of `actions()` (which emits only the nearest
    /// crossing per adjacent map) so `EnterMap { to_position }` can target a particular landing — e.g.
    /// to avoid a dead-end pocket at the nearest crossing (Route 13→14 row 6) — without bloating the
    /// per-step action list (which, emitted per-edge, perturbs `route_toward`/grind navigation).
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

    /// Route the player to a walkable (`Empty`) tile adjacent to `target` and turn to face it.
    /// Returns the button sequence (movement steps + a final turn), which is empty if the player is
    /// already adjacent and facing. Returns `None` if no walkable tile adjacent to `target` is
    /// reachable. Unlike `actions()`, this works for a *dynamic* hidden-object tile whose position
    /// is not known at map-build time (e.g. a gym trash can chosen from RAM).
    pub fn route_to_face(&self, target: Point8) -> Option<Vec<JoypadButton>> {
        self.route_to_face_dir(target, None)
    }

    /// Like `route_to_face`, but if `required` is `Some(dir)` only the approach that ends with the
    /// player facing `dir` is considered. Needed for hidden-object switches (Pokémon Mansion statues)
    /// that only trigger when the player faces them from a specific direction — approaching from any
    /// other adjacent tile faces the wrong way and pressing A does nothing.
    pub fn route_to_face_dir(&self, target: Point8, required: Option<PlayerFacingDirection>) -> Option<Vec<JoypadButton>> {
        let (dist, came_from) = self.bfs_from_player();
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
                    // the player
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
                    MetaTile::CutTree => write!(f, "t")?,
                    MetaTile::Pc      => write!(f, "p")?,
                    MetaTile::Grass   => write!(f, "g")?,
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
mod boulder_solver_tests {
    use super::*;
    use crate::pokemon::sprite::{Sprite, PictureId};

    /// Build a synthetic `MetaTileMap` from ASCII: `#`=wall, `.`=floor, `P`=player, `S`=switch(floor),
    /// `W`=an inter-map warp tile (walkable — the player may stand on it to push), digits `1..9`=boulders.
    /// Returns the map + the switch position.
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
                    'P' => { meta[idx] = MetaTile::Empty; player = p; }
                    'S' => { meta[idx] = MetaTile::Empty; switch = p; }
                    d if d.is_ascii_digit() => {
                        meta[idx] = MetaTile::Empty;
                        sprites.push(Sprite { index: d as u8, picture_id: PictureId::Monster,
                            position: p, on_screen: true, hidden: false,
                            name: Box::leak(format!("Boulder {d}").into_boxed_str()) });
                    }
                    _ => panic!("bad char {c}"),
                }
            }
        }
        (MetaTileMap {
            player_position: player, player_direction: PlayerFacingDirection::Down,
            map: Map::VictoryRoad1F, width: w, height: h, meta_tiles: meta,
            raw_tile_ids: vec![0; w * h], tile_pair_collisions: vec![], sprites,
            warp_targets: HashSet::new(), connection_targets: HashSet::new(),
            spinners: HashMap::new(), can_surf: false,
            strength_switches: vec![switch], holes: vec![],
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
        // Boulder (2,2) → up to (2,1) [player below] → right to (3,1)=switch [player must go AROUND to
        // (1,1)]. Exercises the player-component completeness of the solver.
        let (map, switch) = from_ascii(&["#####", "#..S#", "#.1.#", "#P..#", "#####"]);
        let sol = map.solve_boulder_push(switch).expect("should solve the around-corner push");
        assert_eq!(sol.last().unwrap().1, JoypadButton::Right);
        assert!(sol.len() >= 2, "needs at least two pushes, got {sol:?}");
    }

    #[test]
    fn solves_push_while_standing_on_warp() {
        // The VR1F crux in miniature: the switch (2,1) can only be reached by pushing the boulder
        // (2,2) UP, which requires the player to stand DIRECTLY BELOW it at (2,3) — and that tile is a
        // warp (like VR1F's entrance warp at (8,17)). The player may legitimately stand on it to push.
        // Before the fix the solver excluded warp tiles from standable floor and reported no solution.
        let (map, switch) = from_ascii(&["#####", "#.S.#", "#.1.#", "#PW.#", "#####"]);
        let sol = map.solve_boulder_push(switch).expect("must solve by standing on the warp tile");
        assert_eq!(sol.last().unwrap(), &(Point8 { x: 2, y: 2 }, JoypadButton::Up));
    }

    #[test]
    fn reports_unsolvable() {
        // Boulder walled so it can only wobble left/right in a 1-wide slot, never reaching the switch.
        let (map, switch) = from_ascii(&["#######", "#.....#", "#.###.#", "#.#1#.#", "#.#.#.#", "#..P.S#", "#######"]);
        // The boulder at (3,3) sits in a vertical dead-end; the switch (5,5) is unreachable for it.
        assert!(map.solve_boulder_push(switch).is_none());
    }
}
