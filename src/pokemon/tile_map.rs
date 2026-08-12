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
    /// This map's tileset. Kept so per-tileset ROM tables can be consulted against `raw_tile_ids` —
    /// currently [`crate::pokemon::map_header::TileSetId::warp_tile_ids`], via [`Self::is_step_on_warp`].
    pub tileset: crate::pokemon::map_header::TileSetId,
    /// Unordered raw-tile-ID pairs the player may not walk between in this tileset (elevation
    /// boundaries from pokered `TilePairCollisionsLand`). Empty for most tilesets.
    pub tile_pair_collisions: Vec<(u8, u8)>,
    /// The pairs that apply when **water is on either side** of the move — mounting Surf, stepping
    /// ashore, or moving while surfing (pokered `TilePairCollisionsWater`). In the Cavern tileset
    /// this is `($14, $05)`: inside Seafoam the player can only get on/off the water at a shore
    /// tile, never straight off a plain cave floor.
    pub tile_pair_collisions_water: Vec<(u8, u8)>,
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
    /// True when the player can Surf **here**: Soul Badge, a party mon that knows Surf, and not being
    /// force-ridden on the bike (`IsSurfingAllowed` refuses Surf on Cycling Road, and Routes 16–18 run
    /// along the sea, so believing otherwise routes the BFS straight down the water). When set, the BFS
    /// treats `Water` tiles as passable so routes cross water; the agent mounts Surf at the land↔water
    /// boundary. Set by `game_state()` after construction (the map builder has no party access).
    ///
    /// `IsSurfingAllowed`'s *other* refusal — Seafoam's "current is much too fast" shore tiles — is
    /// modelled separately as [`Self::no_surf_mount`], because it is per-tile rather than per-map.
    pub can_surf: bool,
    /// Strength boulder-switch tiles on this map (invisible pressure plates, from the ROM map scripts):
    /// push a boulder onto one to open its barrier. Exposed so a policy (deterministic or LLM) can
    /// discover *where* to push without hardcoding coordinates. Empty for maps with no Strength puzzle.
    pub strength_switches: Vec<Point8>,
    /// Floor-hole tiles on this map (Victory Road 3F): the player can fall through one to the floor
    /// below, and pushing a boulder onto one drops it there (revealing a hidden boulder). Also modelled
    /// as `MetaTile::Warp` for routing (see `apply_victory_road_holes`); this list is for discovery.
    pub holes: Vec<Point8>,
    /// Land tiles the player may not mount Surf from (Seafoam B4F's (7,11) — "The current is much too
    /// fast!"). One-way: stepping ashore onto them is still allowed. See [`no_surf_mount_table`].
    pub no_surf_mount: HashSet<Point8>,
    /// Hidden items on this map and what each one is: tiles that look like nothing and hand over an
    /// item when faced and pressed A. Nothing on screen marks them, so like [`Self::strength_switches`]
    /// this is exposed so a policy can *discover* them rather than hardcode coordinates. Decoded from
    /// the ROM by [`crate::pokemon::postgame::aides::hidden_items`]; positions are corrected for the
    /// connection strip here, where the raw tables all are. Empty for most maps.
    pub hidden_items: Vec<(Point8, crate::pokemon::item::ItemId)>,
    /// Whether standing in this map's tall grass can produce a wild encounter at all —
    /// `wGrassRate != 0`. See [`CurrentMap::grass_encounter_rate`] for why this is not the same
    /// question as whether the map *has* grass tiles.
    pub has_grass_encounters: bool,
    /// The metadata these tiles were classified from, kept so anything that wants the map's
    /// *pixels* — [`crate::pokemon::map_gfx`] and the picture the model is sent — can reach the
    /// block map, the blockset and the connection strips without re-reading the MMU.
    ///
    /// ⚠️ **The point is that it is the *same* metadata**, not an equivalent one. It carries the
    /// runtime block map for the eleven maps in `map_uses_runtime_blocks` and the door overlays
    /// already applied, so a renderer using it cannot draw a wall where `meta_tiles` says there is
    /// an open door. Re-reading it through `PokemonApi` could, and would also pay
    /// `find_outdoor_entry_map`'s scan of up to 248 map headers.
    ///
    /// ⚠️ `Option` only because this struct derives `Default` (and so does [`GameState`], which
    /// `postgame::fishing`'s tests construct). [`MapMetadata`] has no `Default` and must not gain
    /// one — a default block map is a map of nothing that renders as a plausible empty room.
    pub metadata: Option<std::sync::Arc<crate::pokemon::map_metadata::MapMetadata>>,
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

/// Floor-hole tiles per map (raw coords): a boulder pushed onto one falls to the floor below — and so
/// does the player. Also modelled as `MetaTile::Warp` (see `apply_victory_road_holes` /
/// `apply_seafoam_holes`) so BFS routes through them.
fn hole_table(map: Map) -> &'static [(u8, u8)] {
    match map {
        Map::VictoryRoad3F => &[(23, 15)],
        // pokered `Seafoam{1,2,3,4}HolesCoords`.
        Map::SeafoamIslands1F  => &[(17, 6), (24, 6)],
        Map::SeafoamIslandsB1F => &[(18, 6), (23, 6)],
        Map::SeafoamIslandsB2F => &[(19, 6), (22, 6)],
        Map::SeafoamIslandsB3F => &[(3, 16), (6, 16)],
        _ => &[],
    }
}

/// Land tiles from which the game refuses to let the player *mount* Surf, even though they sit next
/// to water (raw coords). Only Seafoam Islands B4F has one: pokered `IsSurfingAllowed` prints
/// "The current is much too fast!" at (7,11) — the floor's single shore tile — until both boulders
/// have been dropped into the B4F holes. Stepping *ashore* there is still fine, so the restriction is
/// one-way (land → water) and the BFS applies it in that direction only.
fn no_surf_mount_table(map: Map) -> &'static [(u8, u8)] {
    match map {
        Map::SeafoamIslandsB4F => &[(7, 11)],
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
            meta_tiles,
            can_surf: false,
            strength_switches: strength_switch_table(map.metadata.map).iter()
                .map(|&(x, y)| Point8 { x: x + dimensions.west_extra as u8, y: y + dimensions.north_extra as u8 })
                .collect(),
            holes: hole_table(map.metadata.map).iter()
                .map(|&(x, y)| Point8 { x: x + dimensions.west_extra as u8, y: y + dimensions.north_extra as u8 })
                .collect(),
            no_surf_mount: no_surf_mount_table(map.metadata.map).iter()
                .map(|&(x, y)| Point8 { x: x + dimensions.west_extra as u8, y: y + dimensions.north_extra as u8 })
                .collect(),
            hidden_items: crate::pokemon::postgame::aides::hidden_items(map.metadata.map).into_iter()
                .map(|h| (Point8 { x: h.at.x + dimensions.west_extra as u8,
                                   y: h.at.y + dimensions.north_extra as u8 }, h.item))
                .collect(),
            // Already `Arc`'d and cached in `MapMetadataCache`, so this is a refcount bump.
            metadata: Some(std::sync::Arc::clone(&map.metadata)),
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
    /// raw tile IDs form a forbidden pair in this tileset. The check is symmetric, matching
    /// `CheckForTilePairCollisions`.
    ///
    /// Which table applies depends on whether water is involved, exactly as in pokered: moving on
    /// foot uses `TilePairCollisionsLand` (`CollisionCheckOnLand`), while getting on the water
    /// (`UsedSurf`), stepping back off it, and every move made while surfing
    /// (`CollisionCheckOnWater`) use `TilePairCollisionsWater`. So if either end of this edge is a
    /// water tile, the water table governs it.
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

    /// True if the warp on `point` is the kind that fires the moment you **step onto** it
    /// (`CheckWarpsNoCollision`), rather than the map-edge kind that needs the outward direction
    /// pressed. See [`crate::pokemon::map_header::TileSetId::warp_tile_ids`] for why the two cannot be
    /// told apart by position.
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
    /// Every tile the player can **route to** from where they are standing.
    ///
    /// ⚠️ **Not the tiles they can stand on.** This is the key set of [`Self::bfs_from_player`],
    /// which records every neighbour of an open square and only declines to *expand* the ones that
    /// cannot be walked through — because a route has to be allowed to end at a door, a counter, a
    /// cut tree or a person, none of which the player ever occupies. So a wall touching open floor
    /// is in here, and the only things missing are tiles walled in on every side.
    ///
    /// A caller that wants "where can I actually go" has to subtract the walls itself; see
    /// [`crate::llm::map_image`]'s `draw_unreachable`, which shipped the picture of the mistake.
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

    /// Multi-boulder Sokoban: plan a sequence of one-tile pushes that lands *some* boulder on `switch`.
    /// Each entry is `(boulder_position_before_that_push, push_direction)`; returns `None` if no boulder
    /// can reach the switch. Boulders are the sprites named "Boulder …".
    ///
    /// **All** visible boulders move in a single search, which matters wherever one boulder blocks the
    /// player's approach to another. Seafoam B3F is the case that forced it: the boulder at (5,14) seals
    /// the only corridor to (3,14), and (3,14) is the one tile from which the boulder at (3,15) can be
    /// pushed south into the hole at (3,16). A per-boulder search (each treating the rest as walls)
    /// declares that floor unsolvable, which is what it did before this was generalised.
    ///
    /// The state is `(all boulder positions, the player's connected component)` — the component matters
    /// because the same layout is a different position depending on which side of a boulder the player is
    /// stranded on. It is canonicalised by its lexicographically-smallest reachable tile. After a push
    /// the player stands on the pushed boulder's old tile, so the component is recomputed from there.
    ///
    /// Bounded by `MAX_STATES`: BFS over boulder layouts is exponential in the boulder count, and this
    /// runs every agent tick while a boulder step is active. Hitting the cap returns `None` (an
    /// unsolvable-looking floor) rather than growing without limit.
    pub fn solve_boulder_push(&self, switch: Point8) -> Option<Vec<(Point8, JoypadButton)>> {
        use std::collections::{HashMap, HashSet, VecDeque};
        /// Layouts explored before giving up. Real floors settle in the low thousands; the cap only
        /// fires on a pathological map, and keeps the search's memory in the low megabytes.
        const MAX_STATES: usize = 50_000;
        // Only *visible* boulders are physically present and pushable. A hidden boulder (e.g. Victory
        // Road 2F's boulder that stays hidden until a 3F boulder falls through a hole onto it) must be
        // ignored, or the solver plans pushes of a phantom sprite that can never actually move.
        let mut boulders: Vec<Point8> = self.sprites.iter()
            .filter(|s| s.name.starts_with("Boulder") && !s.hidden)
            .map(|s| s.position).collect();
        if boulders.is_empty() { return None; }
        boulders.sort_by_key(|p| (p.y, p.x));   // canonical order, so a layout has one key
        // `self.tile_at` reports the *live* boulder sprites as occupied, but the solver simulates
        // boulders moving — so the tile UNDER any boulder's STARTING position must count as floor (the
        // solver tracks occupancy itself). Without this, a boulder's own starting tile stays a phantom
        // wall after it has (in simulation) been pushed away.
        let initial: HashSet<Point8> = boulders.iter().copied().collect();
        let dirs = [(0i32, -1i32, JoypadButton::Up), (0, 1, JoypadButton::Down),
                    (-1, 0, JoypadButton::Left), (1, 0, JoypadButton::Right)];
        let inb = |x: i32, y: i32| x >= 0 && y >= 0 && (x as usize) < self.width && (y as usize) < self.height;
        let mv = |p: Point8, dx: i32, dy: i32| -> Option<Point8> {
            let (x, y) = (p.x as i32 + dx, p.y as i32 + dy);
            inb(x, y).then(|| Point8 { x: x as u8, y: y as u8 })
        };
        // A tile the player may STAND ON to push a boulder: ordinary floor, and also inter-map warp
        // tiles. The player can legitimately stand on a coordinate-warp (e.g. Victory Road 1F's entrance
        // warps at (8,17)/(9,17), plain cave floor $21) and push a boulder past it — the warp only fires
        // when moving *onto* it toward the warp, not when pushing up into a boulder (VR1F is Cavern, so
        // `ExtraWarpCheck` = `IsWarpTileInFrontOfPlayer`, and the tile in front when pushing is not a
        // warp tile). Excluding warps here was the bug that made VR1F look unsolvable.
        let floor = |p: Point8| initial.contains(&p)
            || matches!(self.tile_at(p), MetaTile::Empty | MetaTile::Grass | MetaTile::Warp { .. });
        // A tile a BOULDER may be pushed onto: ordinary floor (or a tile vacated by a boulder), or the
        // explicit `switch` target itself — this lets the caller aim a boulder at a hole tile (a
        // `MetaTile::Warp`) to drop it to the floor below (Victory Road 3F, Seafoam B3F), which normal
        // floor rules would reject. Any other warp/ladder is off-limits.
        let dest_floor = |p: Point8| p == switch || initial.contains(&p)
            || matches!(self.tile_at(p), MetaTile::Empty | MetaTile::Grass);
        // Tiles the player can reach from `from` with this layout's boulders as walls. Respects
        // tile-pair collisions (cave "cliffs") exactly like real player movement — without this,
        // vacating a boulder could wrongly appear to open a barrier the player can't cross.
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

        type Key = (Vec<Point8>, Point8);           // (boulder layout, player component)
        // The component is only known once a state is popped (it needs a flood fill), so dedup happens
        // at pop time and the queue carries the parent link to record on first arrival.
        let mut visited: HashSet<Key> = HashSet::new();
        let mut came: HashMap<Key, (Key, Point8, JoypadButton)> = HashMap::new();
        let mut q: VecDeque<(Vec<Point8>, Point8, Option<(Key, Point8, JoypadButton)>)> =
            VecDeque::from([(boulders.clone(), self.player_position, None)]);
        while let Some((bs, player_at, parent)) = q.pop_front() {
            let key: Key = (bs.clone(), norm(&reach(&bs, player_at)));
            if !visited.insert(key.clone()) { continue; }
            if let Some(link) = parent { came.insert(key.clone(), link); }
            if bs.contains(&switch) {
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
                    // The player must be able to reach the tile behind the boulder, the destination must
                    // be plain floor, and there must be no elevation/tile-pair cliff between the boulder
                    // and its destination (pokered `CheckForCollisionWhenPushingBoulder`).
                    if !r.contains(&side) || bs.contains(&dest) || !dest_floor(dest) { continue; }
                    if self.pair_blocked(b, dest) { continue; }
                    let mut next = bs.clone();
                    next[i] = dest;
                    next.sort_by_key(|p| (p.y, p.x));
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
                    // Getting ON the water is refused from certain shore tiles (Seafoam B4F's
                    // (7,11), where the current is "much too fast"). One-way — coming ashore there
                    // is fine — so only skip the land → water direction.
                    if self.no_surf_mount.contains(&pos)
                        && matches!(tile, MetaTile::Water | MetaTile::ConnectionWater(_))
                        && !matches!(self.meta_tiles[pos.x as usize + pos.y as usize * self.width],
                            MetaTile::Water | MetaTile::ConnectionWater(_))
                    {
                        continue;
                    }
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

    /// Fixed PC-tile coordinates on this map — see [`pc_locations_for`]. `actions()` emits a
    /// face-and-A route to each.
    fn pc_locations(&self) -> &'static [Point8] {
        pc_locations_for(self.map)
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
        //
        // ⚠️ **Only where something can actually come out of it.** Every town and city draws real
        // tall grass and has `wGrassRate == 0`, so this used to offer an action whose entire purpose
        // — provoke a wild encounter — was impossible to fulfil. Taking it puts the agent into
        // `PacingForEncounters`, which leaves on a map change or on failing to walk, and a healthy
        // pace in empty grass is neither: it wedged the deployed run in Pallet Town for eleven
        // minutes and would have paced there for ever.
        if self.has_grass_encounters && let Some((_, dest)) = nearest(&|t| *t == MetaTile::Grass) {
            let route = reconstruct(dest, &full_from);
            actions.push(OverworldAction { map: self.map, origin: self.player_position, destination: dest, tile: MetaTile::Grass, route });
        }

        // 5. PC tiles (hidden-object interactables): route to the tile below the PC, face up, press A.
        //    Mirrors the sprite-interaction routing (a PC is not a sprite, so it is keyed by fixed
        //    coordinate rather than found in the sprite list).
        for &pc in self.pc_locations() {
            // **The only usable approach is from directly below, facing up.** Not because of the
            // `SPRITE_FACING_UP` in the hidden-event table — that argument does not restrict
            // anything, and the `ANY_FACING` comment in `data/events/hidden_events.asm` says so
            // explicitly; matching is purely on the tile in front of the player. It is the
            // *routines* that check: both `OpenPokemonCenterPC` and `BillsHousePC` open with
            // `ld a, [wSpritePlayerStateData1FacingDirection] / cp SPRITE_FACING_UP / ret nz`.
            //
            // That makes a side approach a silent no-op — the object matches and is dispatched, and
            // the routine returns without drawing anything — so the agent stands there pressing A
            // forever. Approaching a Pokémon Center PC from the west is *nearer* than from below, so
            // a plain nearest-adjacent-tile search picks exactly the one that cannot work.
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

    /// Route to the nearest reachable **water** edge into `to_map` — a `ConnectionWater` tile, crossed
    /// by Surfing off the map edge.
    ///
    /// The companion to [`Self::connection_action`], and needed for the same reason: `actions()` emits
    /// exactly one crossing per adjacent map, the nearest one, so wherever a land bridge and a water
    /// edge both lead to the same map the land bridge always wins and the water edge is unaskable.
    /// Route 24 → Cerulean is the case that motivated it: the footbridge is two steps away, while the
    /// river seam beside it is the *only* way into the half of Cerulean that holds Cerulean Cave.
    ///
    /// `ConnectionWater` carries no landing position (the game decides where you surface), so unlike
    /// `connection_action` there is nothing to disambiguate on — this returns the nearest such edge.
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

/// Fixed PC-tile coordinates on `map` — hidden events the player faces (from below) and presses A on.
///
/// PCs are not derivable from the tileset: a PC is a `hidden_event`, so nothing distinguishes its
/// tile from the wall it is drawn on. This table is transcribed from pokered
/// `data/events/hidden_events.asm`, cross-referenced against `HiddenEventMaps` so each event list is
/// attributed to the right map. That cross-reference is what made it trustworthy when the lists were
/// labelled by hand and the labels had drifted (`SafariZoneRestHouse2` was really
/// `SAFARI_ZONE_WEST_REST_HOUSE`, `CinnabarLab4` really `CINNABAR_LAB_FOSSIL_ROOM`); upstream now
/// keys each list by the map constant itself (`hidden_events_for SAFARI_ZONE_WEST_REST_HOUSE`), so
/// the labels agree with the cross-reference rather than contradicting it.
///
/// Every entry in the file that runs `OpenPokemonCenterPC`, `OpenRedsPC` or `BillsHousePC` is here;
/// there are 22 of them across 21 maps.
pub fn pc_locations_for(map: Map) -> &'static [Point8] {
    /// The overwhelmingly common case: the PC on the back wall of a Pokémon Center, right of the
    /// healing counter. The Celadon Hotel and three of the four Safari rest houses reuse the same
    /// Pokémon-Center layout and so share it.
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
        // The player's own bedroom PC. Note this one runs `OpenRedsPC`, whose menu leads with an
        // ITEM entry the Pokémon-Center PC does not have.
        Map::RedsHouse2F => &[Point8 { x: 0, y: 1 }],
        Map::CeladonMansion2F => &[Point8 { x: 0, y: 5 }],
        Map::IndigoPlateauLobby => &[Point8 { x: 15, y: 7 }],
        // The only map with two: the fossil room's lab machines both open the PC menu.
        Map::CinnabarLabFossilRoom => &[Point8 { x: 0, y: 4 }, Point8 { x: 2, y: 4 }],
        Map::SilphCo11F => &[Point8 { x: 10, y: 12 }],

        _ => &[],
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

    /// Every Pokémon Center has a PC, and it is at the same place in all of them. Before task 0.3 of
    /// the postgame plan, `pc_locations` knew only Bill's house, so the agent could not reach a PC
    /// anywhere it would actually want one.
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
                            facing: crate::pokemon::sprite::SpriteFacing::Down,
                            name: Box::leak(format!("Boulder {d}").into_boxed_str()) });
                    }
                    _ => panic!("bad char {c}"),
                }
            }
        }
        (MetaTileMap {
            player_position: player, player_direction: PlayerFacingDirection::Down,
            map: Map::VictoryRoad1F, width: w, height: h, meta_tiles: meta,
            raw_tile_ids: vec![0; w * h], tileset: crate::pokemon::map_header::TileSetId::Cavern,
            tile_pair_collisions: vec![],
            tile_pair_collisions_water: vec![], sprites,
            warp_targets: HashSet::new(), connection_targets: HashSet::new(),
            spinners: HashMap::new(), can_surf: false,
            strength_switches: vec![switch], holes: vec![], no_surf_mount: HashSet::new(),
            hidden_items: vec![], has_grass_encounters: false,
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
