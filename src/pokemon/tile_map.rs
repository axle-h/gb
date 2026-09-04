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
use crate::pokemon::tile::{HiddenObject, MetaTile};

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
    /// The best fishing rod in the bag, or `None` when there is not one. Set by `game_state()` after
    /// construction for the same reason [`Self::can_surf`] is — the map builder has no bag access —
    /// and read by [`Self::actions`], which offers a `MetaTile::Fish` row only when it is `Some` and
    /// this map has water the player can face.
    pub best_rod: Option<crate::pokemon::postgame::fishing::Rod>,
    /// True when the player can Cut **here**: the Cascade Badge and a party mon that knows Cut
    /// (`GameState::can_use_cut`). Set by `game_state()` after construction, for the same reason
    /// [`Self::can_surf`] is — the map builder has no party access.
    ///
    /// ⚠️ **It gates the cut-tree entries `actions()` emits, and that is not cosmetic.** A `CutTree`
    /// action is a walk that ends *facing* a tree, and the only thing to do from there is the Cut
    /// field move — which, without the move or the badge, opens the party menu onto a mon that has
    /// no CUT entry and never comes back out. The deployed run found it: eleven turns on Route 2 with
    /// no badges at all, `cut got no answer from the game for 60s` each time, and the model
    /// reasonably concluded the game was broken. An action nobody can carry out is worse than a
    /// missing one, so it is not offered.
    pub can_cut: bool,
    /// True when the player can use **Strength** here: the Rainbow Badge and a party mon that knows
    /// it. Set by `game_state()` after construction, for the same reason [`Self::can_cut`] is.
    ///
    /// ⚠️ **It gates the boulder rows `actions()` emits, and it is the same argument `can_cut`
    /// carries one obstacle along.** A push row is a walk to the square beside a boulder followed by
    /// a shove; without the move or the badge the shove moves nothing and says nothing
    /// (`TryPushingBoulder` returns on the very first `bit BIT_STRENGTH_ACTIVE`), so the row is an
    /// invitation into a silent sixty-second stall. What the turn says instead is that the boulders
    /// are there and which half is missing — see `prompt::situation`.
    ///
    /// ⚠️ **Not the same question as `GameState::strength_active`**, which is whether Strength has
    /// been armed from the party menu *on this map* and is cleared by every map change. That one is
    /// the driver's business: `AgentState::PushingBoulder` arms it itself.
    pub can_strength: bool,
    /// Is Bill waiting inside his own machine?
    ///
    /// ⚠️ **The one hidden object in the table that is gated, and the gate is the whole point.**
    /// `EVENT_BILL_SAID_USE_CELL_SEPARATOR` is set and `EVENT_USED_CELL_SEPARATOR_ON_BILL` is not,
    /// which is exactly the window in which pressing that PC does something. Outside it the press
    /// opens a storage menu with no tool behind it, which is the row `overworld_menu` withholds
    /// `MetaTile::Pc` for. Same shape as [`Self::can_cut`], and on the map rather than in
    /// `llm::tools` for the same reason: one place to be right rather than two.
    pub bill_cell_separator: bool,
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
    // ⚠️ **There was a `hidden_items` here**, decoded from the ROM's own two tables and corrected
    // for the connection strip. It is gone with the rest of hidden-item collection (2026-09-03): the
    // policy step that read it went, and so did the `interact` tool that shared its driver. See
    // `crate::pokemon::postgame::aides`' module docs for why. Nothing in the game is behind a hidden
    // item, so no route lost anything.
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


/// How the cartridge can be made to take a warp entry the player is standing on. See
/// [`MetaTileMap::warp_trigger`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarpTrigger {
    /// The tile is a door or a warp tile in its own right: arriving on it warps, no button needed.
    StepOn,
    /// It warps only while this direction is held, either as the last step of the walk onto it or as
    /// a bump into the wall from on top of it.
    HoldDirection(JoypadButton),
    /// Nothing triggers it from this side. The warp entry exists so that *leaving* the far map lands
    /// the player here; it is not a way in.
    Impossible,
    /// The check depends on a tile this model does not hold, so nothing is claimed either way.
    ///
    /// ⚠️ **`_GetTileAndCoordsInFrontOfPlayer` reads the on-screen tilemap, not the map.** A player
    /// on the edge of a map facing out is looking at the **border block**, which is a real tile with
    /// a real id and is drawn from the map header's border byte rather than from the block map. So
    /// an entry on row 0 or on the last column cannot be proved dead from `raw_tile_ids` alone, and
    /// several real doors sit exactly there: the S.S. Anne's gangway to Vermilion Dock at (26, 0),
    /// Rock Tunnel 1F's north mouth, and the front door of Cerulean's badge house, whose SHIP
    /// tileset sends it down the tile-in-front arm that a house would not have taken. Calling those
    /// impossible would have taken the only door out of each of them.
    Unknown,
}

/// One way off this map into an adjacent one: a **run of touching edge tiles**, not a tile.
///
/// ⚠️ **The group is the unit because the tiles are not choices.** A map's border strip into its
/// neighbour can be dozens of tiles wide, and stepping onto any tile of one run lands the player in
/// the same place, so listing them individually would be forty rows of one decision. What *is* a
/// decision is which run. Route 14's east edge is open at rows 0, 1, 2, 4, 6, 8 and 10; row 6 is a
/// six-tile pocket whose only way west is through a trainer's body, and rows 4 and 8 are the road.
/// The deployed run of 2026-09-02 crossed into that pocket, walked back to Route 13, crossed again
/// and landed in it a second time, because the nearest crossing was the only one anything offered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Crossing {
    /// The tile to leave by, in this map's action-id coordinates. The reachable member of the run
    /// nearest the player when there is one, so the id names a square that can actually be walked to.
    pub at: Point8,
    /// Where it lands on the far map, in that map's raw coordinates. Paired with `at` because
    /// [`MetaTileMap::connection_action`] is keyed on it, and two runs into the same map differ only
    /// here.
    pub to_position: Point8,
    /// Whether any tile of the run can be walked to from where the player is standing.
    ///
    /// ⚠️ **This is the field the whole type exists for.** `connection_targets` is header
    /// connectivity: it says Cerulean touches Route 5, which is true and was not the question. The
    /// question is whether the player, standing on the terrace they are standing on, can get to the
    /// edge that leads there, and for 65 turns of the deployed run the answer was no while every
    /// read said yes.
    pub reachable: bool,
    /// How many tiles the run holds. Only used to keep the wider run first when two are otherwise
    /// equal, on the reasoning that a one-tile gap in a wall is more often a pocket than a road.
    pub tiles: usize,
}

/// What the search charges for a step from land onto water, in walking steps.
///
/// ⚠️ **Measured, not picked.** A Surf mount is the whole START→POKéMON→mon→SURF menu chain plus the
/// cartridge's own mount animation and the scripted step onto the water: about 150 agent ticks
/// against roughly 14 for one walking step, so ten steps is what it actually costs, and the encounter
/// roll on that scripted step is thrown in free. The number decides one thing — how far the search
/// will walk round a piece of water rather than get on it — and both sides of it are load-bearing.
/// Too low and it goes back to crossing Route 21's islands and Cinnabar's harbour a mount at a time.
/// Too high and it walks the long way round a lake that Surf crosses in three tiles, which is slower
/// in exactly the way this is meant to avoid.
const SURF_MOUNT_COST: u32 = 10;

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
        // ⚠️ **Steps, not the price `bfs_from_player` reports.** This wants the tile that takes the
        // most *walking* to reach, because walking is what rolls for an encounter; measured on the
        // price, every water tile would gain [`SURF_MOUNT_COST`] and a mixed map would send the
        // trainee out to sea to pace on the nearest wave instead of walking the floor.
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

    /// The raw tile id `CheckForCollisionWhenPushingBoulder` refuses a boulder onto **by id**, as a
    /// special case sitting beside the tileset's own collision list. It is compared regardless of
    /// tileset, and in the only ones a boulder is ever on — every Strength puzzle in the game is
    /// Cavern, on Victory Road and in the Seafoam Islands — it is the staircase, which is why the
    /// refusal calls it one. Stairs are walkable, so nothing else about the tile says no: this is
    /// the constant that cost a deployed run its Victory Road, at (5, 13) on VictoryRoad1F, one
    /// square north of the boulder at (5, 14).
    const BOULDER_STAIRS_TILE: u8 = 0x15;

    /// The two refusals a boulder push gets **that ordinary walking does not**, given the tile the
    /// player would be standing on and the tile the boulder would be pushed onto: `Some(reason)` if
    /// the cartridge would refuse.
    ///
    /// One implementation with two callers that need different halves of it — `solve_boulder_push`
    /// wants the yes/no, because it simulates layouts the live map does not have and so cannot ask
    /// [`Self::boulder_push_refusal`]; the refusal wants the sentence. They must never drift, which
    /// is the whole reason this is not written out twice.
    ///
    /// ⚠️ **The tile pair the cartridge tests is (the player's tile, the destination), which are two
    /// squares apart.** `CheckForCollisionWhenPushingBoulder` calls `GetTileTwoStepsInFrontOfPlayer`
    /// — which overwrites `wTileInFrontOfPlayer` with the tile two ahead — and then
    /// `CheckForTilePairCollisions2`, which compares that against `wTilePlayerStandingOn`. The
    /// boulder's own tile is never in it. This used to test (boulder, destination), which is a
    /// different pair on any cliff a boulder is standing on the edge of.
    fn boulder_push_terrain_refusal(&self, stand: Point8, dest: Point8) -> Option<String> {
        // Two rules, two sentences: they are refused at the same moment and are nothing alike, and a
        // model told "a step up or down" about a staircase will look for a cliff that is not there.
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
    ///
    /// ⚠️ **Visible ones only.** A hidden boulder (Victory Road 2F keeps one until a 3F boulder
    /// falls through a hole onto it) is not physically there, so naming it in the turn would point
    /// the model at a square with nothing on it — the same rule `solve_boulder_push` filters on.
    pub fn boulders(&self) -> Vec<Point8> {
        let mut found: Vec<Point8> = self.sprites.iter()
            .filter(|s| s.name.starts_with("Boulder") && !s.hidden)
            .map(|s| s.position).collect();
        found.sort_by_key(|p| (p.y, p.x));
        found
    }

    /// Every one-tile shove on this map the cartridge would actually carry out, as
    /// `(boulder, direction, the square it is pushed from)`.
    ///
    /// ⚠️ **This is `actions()`'s boulder section and it must stay one flood fill.** Nine boulders
    /// on a Victory Road floor is thirty-six questions, each of which wants to know whether the
    /// square it would be pushed from can be walked to; `boulder_push_refusal_inner` therefore takes
    /// the reachable set rather than computing one, and this is where it is computed once. The same
    /// lesson `nearest_castable_water` cost a whole deployed run of stutter — see its ⚠️.
    pub fn boulder_pushes(&self) -> Vec<(Point8, JoypadButton, Point8)> {
        self.boulder_pushes_within(&self.reachable_tiles())
    }

    /// [`Self::boulder_pushes`] against a reachable set already computed, which is what `actions()`
    /// calls: it has run `bfs_from_player` at the top of itself and a second flood fill per tick is
    /// the cost `nearest_castable_water` learned about the hard way.
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
    ///
    /// ⚠️ **A refused push is silent, and silence is a sixty-second stall.** pokered's
    /// `TryPushingBoulder` answers every one of these by falling into `ResetBoulderPushFlags` and
    /// returning: no message, no animation, nothing on screen at all. `AgentState::PushingBoulder`
    /// completes only when the boulder leaves its tile, so it holds the direction until
    /// `agent::DRIVER_ESCAPE_SILENCE` and reports "got no answer from the game for
    /// 60s", which reads as a malfunction — the same mistake `teach_refusal` and
    /// `item_use::field_use_refusal` were written to stop making. The deployed run of 2026-09-02
    /// filed five issue reports about it against the one boulder at (5, 14) on VictoryRoad1F, whose
    /// north neighbour is a staircase, and got no further into Victory Road.
    ///
    /// Every rule is a conjunct, so the order is only about which sentence comes back first: the
    /// destination has to be on the map, then clear of every other sprite and passable in this
    /// tileset, then neither of [`Self::boulder_push_terrain_refusal`]'s two, and finally the tile
    /// the shove happens from has to be walkable to.
    pub fn boulder_push_refusal(&self, boulder: Point8, dir: JoypadButton) -> Option<String> {
        let name = self.sprites.iter()
            .find(|s| s.name.starts_with("Boulder") && !s.hidden && s.position == boulder)
            .map(|s| s.name.clone());
        let Some(name) = name else {
            return Some(format!(
                "There is no boulder at ({}, {}). `read_map` gives the exact position of every one \
                 on this map.", boulder.x, boulder.y));
        };
        // One flood fill for all four directions: the "and these do work" clause asks about three
        // more pushes, and each of them needs to know whether the tile it would be shoved from can
        // be walked to.
        let reach = self.reachable_tiles();
        let refused = |why: String| {
            // ⚠️ **Naming the directions that do work is the half that matters.** A bare "no" leaves
            // a model with four squares to guess between and nothing said about which, and the last
            // one it tried is the one it reads back on its next request; a run that is told "east or
            // west" pushes east. `teach_refusal` names who *can* take the move for the same reason.
            let ways: Vec<&str> = [(JoypadButton::Up, "up"), (JoypadButton::Down, "down"),
                                   (JoypadButton::Left, "left"), (JoypadButton::Right, "right")]
                .into_iter()
                .filter(|(d, _)| *d != dir && self.boulder_push_refusal_inner(boulder, *d, &reach).is_none())
                .map(|(_, w)| w).collect();
            let rest = match ways.as_slice() {
                // ⚠️ **A boulder with nowhere to go is a *reset*, not a dead end, and saying only
                // the first half is how a model decides the game is broken.** Gen 1 keeps nothing
                // about where a boulder has been shoved to: the sprites come off the map's own
                // object data every time `LoadMapData` runs, so walking out and back undoes every
                // push on the floor. `endgame::leaving_a_map_puts_its_boulders_back` is the proof,
                // taken on the save state of the run that needed to be told this.
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
        // The tile the player has to stand on to shove it, one square the other way. If that is off
        // the map there is nowhere to push from and the shove can never happen.
        let Some(stand) = self.step(boulder, opposite_dir(dir)) else {
            return Some(format!("there is nowhere to stand on the far side, at the edge of {}", self.map));
        };
        match self.tile_at(dest) {
            // A hole is a legitimate destination — Victory Road 3F and Seafoam B3F are solved by
            // dropping a boulder through one — so a warp tile is passable here where it is not for
            // `dest_floor`'s ordinary floor.
            MetaTile::Empty | MetaTile::Grass | MetaTile::Warp { .. } => {}
            MetaTile::Sprite(who) => return Some(format!("{who} is standing in the way at ({}, {})", dest.x, dest.y)),
            other => return Some(format!("({}, {}) is {other}", dest.x, dest.y)),
        }
        if let Some(refusal) = self.boulder_push_terrain_refusal(stand, dest) {
            return Some(refusal);
        }
        // ⚠️ **Not one of the cartridge's checks, and it has to be here anyway.** A shove happens by
        // walking into the boulder, so a push tile the player cannot get to is a push that can never
        // be attempted — `AgentState::PushingBoulder` finds no route, drops to `Idle` without a
        // word, and the policy asks for the identical push again. It is also the difference between
        // a boulder that is merely awkward and one that is *stuck*, which on a Strength puzzle is
        // the whole answer: on VictoryRoad1F a boulder shoved north into the alcove at (5, 14) seals
        // the only way to the tile it would have to be pushed back from.
        //
        // ⚠️ **Two questions, and asking only the second one let the bug straight through.**
        // [`Self::reachable_tiles`] is the key set of `bfs_from_player`, which records every
        // *neighbour* of an open square and declines only to expand the ones that cannot be walked
        // through — its own doc comment says so — because a route has to be allowed to end at a
        // door, a counter or a person. So a wall touching floor is in it, and `reach.contains` alone
        // says yes to standing inside the wall. A deployed run was offered, and accepted, a push
        // left on a boulder whose right-hand neighbour was solid rock: there was nowhere to stand,
        // the shove was never attempted, and the silence was the sixty seconds this whole function
        // exists to avoid. The tile has to be one the player can *occupy* first, which is the same
        // `floor` predicate `solve_boulder_push` uses (a coordinate warp is floor: VR1F's entrance
        // warps are plain cave floor and a boulder is legitimately pushed past them).
        match self.tile_at(stand) {
            MetaTile::Empty | MetaTile::Grass | MetaTile::Warp { .. } => {}
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
                    // be plain floor, and the two extra refusals pokered
                    // `CheckForCollisionWhenPushingBoulder` adds — a staircase, and an elevation
                    // boundary — must not apply.
                    if !r.contains(&side) || bs.contains(&dest) || !dest_floor(dest) { continue; }
                    // ⚠️ **`side`, not `b`** — the cartridge tests the player's tile against the
                    // destination, and the staircase rule with it. See
                    // [`Self::boulder_push_terrain_refusal`].
                    if self.boulder_push_terrain_refusal(side, dest).is_some() { continue; }
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

    /// One [`Self::bfs_from_player`], for a caller that is about to ask
    /// [`Self::route_to_face_within`] about many targets. See that method for why the loop must not
    /// run its own search per target.
    pub fn search_for_faces(&self) -> (HashMap<Point8, u32>, HashMap<Point8, (Point8, JoypadButton)>) {
        self.bfs_from_player()
    }

    /// Search from `player_position` outward.
    ///
    /// Returns `(dist, came_from)` where `dist[p]` is the cheapest way to reach `p` and
    /// `came_from[p]` is the `(previous_position, direction)` before it on that route.
    ///
    /// ⚠️ **`dist` is a *price*, not a step count, and the only thing that is not free is getting on
    /// the water.** Every ordinary step costs 1; a step from land onto `Water` or `ConnectionWater`
    /// costs [`SURF_MOUNT_COST`] more, because the agent has to stop and drive
    /// START→POKéMON→mon→SURF before it can take it. Callers that ask "which of these is nearest"
    /// (`actions`, `crossings`, `connection_action`, `route_to_face_dir`) all want that price rather
    /// than a step count — a land bridge two steps further off beats a river seam — so they read this
    /// unchanged. [`Self::wander_action`] is the one caller that genuinely means *steps*, and it takes
    /// them from [`Self::search_from_player`]'s third map instead.
    ///
    /// ⚠️ **A bucket queue, so a map with no water routes exactly as it did under the plain BFS this
    /// replaced.** Bucket *n* holds the frontier at price *n* and is drained FIFO, so when every edge
    /// costs 1 the buckets are the BFS layers and the order within one is the BFS queue order — and
    /// `came_from` is written on first discovery in both. That property is the whole reason for the
    /// bucket queue over a binary heap, whose tie-breaking would quietly re-shape every route on every
    /// map in the game.
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

        // If the player is standing on an arrow tile (mid-slide), the forced movement will carry them
        // to its rest destination — start the search from there.
        let start = self.resolve_spinner(self.player_position);
        dist.insert(start, 0);
        steps.insert(start, 0);
        buckets[0].push_back(start);

        // Record the edge into `to` if it is the cheapest way there so far, and say whether it was.
        // A terminal tile (a door, a wall, a person) goes through this too — it is a place a route may
        // *end* — and the caller then declines to queue it.
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

        // Buckets are grown on demand: the highest price any tile can carry is one step per square
        // plus one mount per land→water boundary crossed, which is bounded but not worth computing.
        fn push(buckets: &mut Vec<VecDeque<Point8>>, price: u32, p: Point8) {
            let i = price as usize;
            if buckets.len() <= i { buckets.resize(i + 1, VecDeque::new()); }
            buckets[i].push_back(p);
        }

        let mut bucket = 0usize;
        while bucket < buckets.len() {
            while let Some(pos) = buckets[bucket].pop_front() {
                // A stale copy: this tile was queued at this price and then found more cheaply, or it
                // has already been expanded. Either way there is nothing left to do with it.
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
                    if settled.contains(&nb) { continue; }

                    // Arrow (spinner) tile: stepping onto `nb` hands control to the game, which slides the
                    // player to a fixed destination. Record an edge from `pos` (press `dir`) → that
                    // destination; the player never stops on the arrow itself.
                    if self.spinners.contains_key(&nb) {
                        let dest = self.resolve_spinner(nb);
                        if relax!(pos, dest, dir, 1) { push(&mut buckets, dist[&dest], dest); }
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
                            if (dest.x as usize) < self.width && (dest.y as usize) < self.height
                                && relax!(pos, dest, dir, 1)
                            {
                                push(&mut buckets, dist[&dest], dest);
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
                                if !landing_blocked && relax!(pos, landing, dir, 1) {
                                    push(&mut buckets, dist[&landing], landing);
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
                            && !matches!(here, MetaTile::Water | MetaTile::ConnectionWater(_))
                        {
                            continue;
                        }
                        // ⚠️ **Getting on the water is the one step that is not one step.** Walking
                        // from land onto water means stopping to drive START→POKéMON→mon→SURF, which is
                        // about [`SURF_MOUNT_COST`] walking steps of game time, plus the encounter roll on
                        // the auto-step the mount ends with. Priced at 1, the search took every water
                        // short cut it was offered and paid a mount for it: the deployed run of
                        // 2026-09-03 crossed Route 21 straight up x = 7, which runs over the two little
                        // islands at y = 25/26, so it dismounted onto sand and remounted twice for a
                        // detour of *two tiles* — and each remount, before `Surfing::resume`, also threw
                        // the walk away and cost a request. Water→water and water→land stay at 1: coming
                        // ashore is free, and a player already surfing pays nothing to carry on.
                        let mounting = !matches!(here, MetaTile::Water | MetaTile::ConnectionWater(_))
                            && matches!(tile, MetaTile::Water | MetaTile::ConnectionWater(_));
                        let edge = if mounting { 1 + SURF_MOUNT_COST } else { 1 };
                        if !relax!(pos, nb, dir, edge) { continue; }
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
                            push(&mut buckets, dist[&nb], nb);
                        }
                    }
                }
            }
            bucket += 1;
        }
        (dist, steps, came_from)
    }

    /// Fixed PC-tile coordinates on this map — see [`pc_locations_for`]. `actions()` emits a
    /// face-and-A route to each.
    fn pc_locations(&self) -> &'static [Point8] {
        pc_locations_for(self.map)
    }

    /// Does this map draw tall grass anywhere, reachable or not?
    ///
    /// ⚠️ **"Has grass" and "has encounters" are different questions and confusing them loops the
    /// grind for ever.** A cave has `has_grass_encounters` true and no `MetaTile::Grass` at all,
    /// because pokered points `wGrassTile` at the cave floor and every step rolls; a route has both.
    /// So a route where no grass is *reachable* is a map the trainee cannot be levelled on, while a
    /// cave with no grass is the ordinary case — and only this tells them apart.
    pub fn has_grass_tiles(&self) -> bool {
        self.meta_tiles.iter().any(|tile| *tile == MetaTile::Grass)
    }

    /// Fixed hidden-object sites on this map — see [`hidden_objects_for`]. `actions()` emits a row
    /// per reachable one, the same way it does for a PC.
    fn hidden_objects(&self) -> &'static [HiddenObjectSite] {
        hidden_objects_for(self.map)
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

        // ⚠️ **W5 — a warp entry the cartridge will not open is worse than no row.** See
        // [`Self::warp_trigger`]: Route 8's east gate has two entries and only one of them is a way
        // in. The dud is dropped **only when another warp on this map leads to the same place**, and
        // that guard is not caution for its own sake: `warp_trigger` is a transcription of
        // `home/overworld.asm` and a false negative in it would take away the only door out of
        // somewhere and strand the run for good. With the guard the worst a mistake can cost is a
        // row that was already useless.
        // ⚠️ **Counted over warps that can actually be *opened*, not over warps that exist.** The
        // first draft counted entries, and Cerulean's badge house has two: a front door and a back
        // door. Both looked impossible (its SHIP tileset sends a house down the tile-in-front arm,
        // and both are on the map's edge where this model cannot see the tile in front), so each one
        // was dropped because the other existed and the house had no exit at all. A warp is only
        // ever given up in favour of one that is known to work.
        let ways_to: HashMap<Map, usize> = self.meta_tiles.iter().enumerate()
            .filter_map(|(index, tile)| match tile {
                MetaTile::Warp { to_map, .. } => {
                    let at = Point8 { x: (index % self.width) as u8, y: (index / self.width) as u8 };
                    matches!(self.warp_trigger(at),
                             WarpTrigger::StepOn | WarpTrigger::HoldDirection(_)).then_some(*to_map)
                }
                _ => None,
            })
            .fold(HashMap::new(), |mut counts, to_map| {
                *counts.entry(to_map).or_default() += 1;
                counts
            });
        for (warp_to_map, warp_to_pos) in &self.warp_targets {
            let Some((tile, dest)) = nearest(&|t| matches!(t, MetaTile::Warp { to_map, to_position } if to_map == warp_to_map && to_position == warp_to_pos)) else { continue };
            let trigger = self.warp_trigger(dest);
            if trigger == WarpTrigger::Impossible
                && ways_to.get(warp_to_map).copied().unwrap_or(0) > 0 { continue }
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
                    // ⚠️ **One held button, not a step off and a step back.** Standing on the entry
                    // with `BIT_STANDING_ON_WARP` already set, walking into the wall in front is a
                    // *collision* on a warp tile, which `home/overworld.asm` sends straight to
                    // `ExtraWarpCheck` and `CheckWarpsCollision`. It needs no room to step into, and
                    // it survives the route being re-derived every tick — which the step-off dance
                    // did not, because each tick recomputed a two-step route and only ever pressed
                    // its head. That is the shuffle a deployed run did for 60 s at the Route 8 gate.
                    WarpTrigger::HoldDirection(dir) => route.push(dir),
                    // A door tile warps on the step onto it and needs no direction, so step off to a
                    // genuinely walkable neighbour and step back. A real `Empty` neighbour, because
                    // defaulting to Down can walk into a wall and jam.
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
        //
        //    ⚠️ **A water crossing is only offered when the player can Surf.** `ConnectionWater` is
        //    reachable from the shore whether or not anything in the party can mount it — the BFS
        //    records it as a terminal neighbour — so without Surf the row is a walk to the water's
        //    edge and a bump into the sea. Same rule as the cut trees below, for the same reason.
        for to_map in &self.connection_targets {
            let Some((tile, dest)) = nearest(&|t| match t {
                MetaTile::Connection { to_map: m, .. } => m == to_map,
                MetaTile::ConnectionWater(m) => self.can_surf && m == to_map,
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

        // 6. Fishing — stand on the shore, face the water, cast.
        //
        // ⚠️ **Three gates, and each one is a row the game would refuse.** A rod in the bag, because
        // `FishingInit` needs the item; a tileset in `WaterTilesets`, because the ROM checks that
        // before it looks at the tile in front and a map that fails it answers every cast with "Not
        // the time to use that!"; and water with a reachable `Empty` neighbour, because a cast is
        // made from land. That is the same rule the `CutTree` and `ConnectionWater` rows keep, for
        // the reason the system prompt gives: an action the cartridge silently declines is worse
        // than no action, because nothing about the refusal tells the policy to stop asking.
        //
        // ⚠️ **One row, not one per shore.** Every other section here emits the nearest of its kind
        // and this is no different: which puddle is cast into changes nothing, since the fishing
        // group is per *map*.
        //
        // The route ends facing the water and there is no `A`: pressing A at water does nothing, and
        // the cast is a bag chain the `Fishing` driver owns. The agent picks that driver up when the
        // walk arrives — see `AgentState::OverworldMovement`'s empty-route arm.
        if let Some(rod) = self.best_rod
            && crate::pokemon::postgame::fishing::tileset_holds_water(self.tileset)
            && let Some(water) = crate::pokemon::postgame::fishing::nearest_castable_water(self)
        {
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

        // 5b. Hidden objects — gym bins, vending machines, the Game Corner poster, Mansion statues.
        //     Routed exactly like a PC because the cartridge dispatches them the same way
        //     (`CheckForHiddenEvent` on the tile in front of the player), with one difference: a PC
        //     is only ever approached from below, and these carry the approach the routine behind
        //     them actually demands. A statue checks `SPRITE_FACING_UP` like the PCs do; a bin and a
        //     bg-event sign check nothing, so any side that can be reached will do.
        //
        //     ⚠️ **Not derived from the tileset, for the same reason `pc_locations_for` is not.** A
        //     hidden object is drawn as the wall it is hiding in, so nothing in the block map tells
        //     one from ordinary scenery; the table is transcribed and tested against the
        //     disassembly.
        for (index, site) in self.hidden_objects().iter().enumerate() {
            if site.object == HiddenObject::CellSeparator && !self.bill_cell_separator { continue }
            // Numbered within this map's table, over the whole table rather than per object kind, so
            // a bin's ordinal is its `wGymTrashCanIndex` plus one and a row's number never shifts
            // because something unrelated was added beside it.
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

        // 6. Cut trees: route to a walkable tile adjacent to a CutTree and face it (no A), and let
        //    `AgentState::CuttingTree` do the cut — the row is the whole thing, exactly as a boulder
        //    row and a fishing row are. One action per reachable-adjacent tree, and each carries the
        //    tree it is about (`MetaTile::Cut { at }`) rather than sharing one anonymous tile: see
        //    the ⚠️ on that variant for what the shared one cost.
        //
        //    ⚠️ **Only when Cut can actually be used** — see [`Self::can_cut`]. The action ends facing
        //    a tree and nothing else, so without the move and the badge it is an invitation into a
        //    party menu with no CUT in it.
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

        // 7. Boulder pushes: route to the square a boulder can be shoved from, face it, and let
        //    `AgentState::PushingBoulder` do the rest — including arming Strength, which is why
        //    there is no separate row and no separate tool call for that.
        //
        //    ⚠️ **Only pushes the cartridge would actually carry out**, which is the whole of
        //    [`Self::boulder_push_refusal`] asked of all four directions at once. A refused shove is
        //    refused in *silence*, so a row for one is sixty seconds of holding a direction at a
        //    boulder that will never move; that is the stall the deployed run of 2026-09-02 filed
        //    five issue reports about, and offering the refusals as menu rows would be the same bug
        //    with a nicer interface. The rows that survive are the decisions a player actually has.
        //
        //    ⚠️ **And only when Strength can be used at all** — see [`Self::can_strength`], the same
        //    rule the cut trees above keep.
        //
        //    ⚠️ **One flood fill for every boulder on the floor.** `boulder_pushes` computes the
        //    reachable set once and asks all four directions of every boulder against it; this runs
        //    on every 20 ms agent tick, and Victory Road 2F has three boulders.
        //
        //    The sprite scan comes first because the set below is a copy of the whole reachable
        //    region, and past the Rainbow Badge `can_strength` is true on every map in the game
        //    while boulders are on five of them.
        if self.can_strength && self.sprites.iter().any(|s| !s.hidden && s.name.starts_with("Boulder")) {
            let reach: std::collections::HashSet<Point8> = full_dist.keys().copied().collect();
            for (boulder, push, stand) in self.boulder_pushes_within(&reach) {
                let Some((_, came_from)) = best_dist_from(&stand) else { continue };
                let mut route = reconstruct(stand, came_from);
                // Facing is not optional: `TryPushingBoulder` reads
                // `wSpritePlayerStateData1FacingDirection` and the push needs the direction held
                // twice, so the route ends turned toward the boulder exactly as a cut tree's does.
                // The driver re-derives all of this every tick anyway — this is what the *menu* row
                // promises, not what carries it out.
                if route.is_empty() {
                    let facing: JoypadButton = self.player_direction.into();
                    if facing != push { route.push(push); }
                } else if route.last() != Some(&push) {
                    route.push(push);
                }
                actions.push(OverworldAction { map: self.map, origin: self.player_position,
                    destination: stand, tile: MetaTile::Boulder { at: boulder, push }, route });
            }
        }

        actions.sort();
        actions
    }

    /// What it takes to make the warp entry at `at` actually fire, as the cartridge decides it.
    ///
    /// ⚠️ **W5 — a warp entry is not the same thing as a door, and the difference stalled a
    /// deployed run for a minute of game time at a gate it was standing on.** `home/overworld.asm`
    /// only warps a player already on a warp entry if one of two things holds:
    /// `IsPlayerStandingOnDoorTileOrWarpTile`, which is the tile's own id against the tileset's list
    /// ([`TileSetId::warp_tile_ids`]) and fires with no button at all; or `ExtraWarpCheck`, which
    /// needs a direction held *and* either the tile in front to be a warp carpet
    /// ([`TileSetId::warp_carpet_tile_ids`], "function 2") or the player to be at the edge of the
    /// map facing out ("function 1"). Route 8's two east-gate entries are $2C at (9, 9) and $39 at
    /// (9, 10); neither is a door tile, the tile west of (9, 10) is a carpet and the tile west of
    /// (9, 9) is not, so one of the two doors on that gate is a door the game will not open.
    ///
    /// ⚠️ **This is a *sufficient* condition for the trigger, not for arriving.** It says nothing
    /// about whether the player can walk to `at`; that is the BFS's job.
    pub fn warp_trigger(&self, at: Point8) -> WarpTrigger {
        let Some(&here) = self.raw_tile_ids.get(at.x as usize + at.y as usize * self.width) else {
            return WarpTrigger::Impossible;
        };
        if self.tileset.warp_tile_ids().contains(&here) {
            return WarpTrigger::StepOn;
        }
        // `ExtraWarpCheck`'s dispatch, in its own order: SS Anne 3F takes function 1 whatever its
        // tileset says, four named maps take function 2 whatever theirs says, and only then does the
        // tileset decide.
        let reads_the_tile_in_front = match self.map {
            Map::SSAnne3F => false,
            Map::RocketHideoutB1F | Map::RocketHideoutB2F | Map::RocketHideoutB4F
            | Map::RockTunnel1F => true,
            _ => self.tileset.warp_check_reads_the_tile_in_front(),
        };
        if !reads_the_tile_in_front {
            // `IsPlayerFacingEdgeOfMap`: at the edge, facing out. Every map this arm covers is an
            // interior, which has no connection strips, so the padded grid and the raw one agree.
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

    /// Every distinct way off this map into `to_map`, one [`Crossing`] per run of touching edge
    /// tiles, nearest-reachable first and then in reading order.
    ///
    /// ⚠️ **Land crossings only.** A `ConnectionWater` seam has no `to_position` to disambiguate on
    /// (the game decides where the player surfaces), it is offered by
    /// [`Self::water_connection_action`] instead, and without Surf it is scenery rather than a way
    /// out. Counting one here would report a shore as a crossing the player cannot reach, which is
    /// the false alarm the `Blocked here: Water` line was deleted for.
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
            // reached at all, otherwise the first in reading order. Naming an unreachable member of
            // a reachable run would mint an id `connection_action` then declines to route to.
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

    /// What the region the player can walk in **ends on**: the kinds of impassable tile that touch
    /// it, commonest first, as noun phrases fit to drop into a sentence.
    ///
    /// ⚠️ **Read off the BFS's own key set, which is why it costs nothing.** `reachable_tiles` is
    /// deliberately the set of squares the search *touched*, walls included — a route has to be
    /// allowed to end at a door, a counter, a tree or a person — so the impassable members of it are
    /// exactly the wall of the room, already computed. See that method's ⚠️.
    ///
    /// ⚠️ **Plain walls are not reported.** Every region in the game is bounded by scenery, so
    /// "walls" is true everywhere and answers nothing; what the model can act on is the boundary it
    /// might get *through* — a tree that Cut clears, a ledge that only goes one way, water that
    /// needs Surf, somebody standing in the gap.
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

    /// What an `A` press here would actually talk to: the tile in front, or the one *behind* it
    /// when that is a [`MetaTile::Counter`] and a person is standing there.
    ///
    /// ⚠️ **Gen 1 talks over a counter** — `wTilesetTalkingOverTiles`, which is what `Counter` is
    /// read from — and that is how a Pokémon Centre nurse, a mart clerk, a gym receptionist and
    /// every desk in the game are spoken to. The player is never adjacent to one of them, so
    /// [`Self::tile_in_front`] answers `Counter` and anything asking "is the thing I walked over
    /// for what I am now facing" gets `false` for every conversation held across a desk.
    /// [`Self::actions`] already routes to the far side (its `counter_extra` positions), so the
    /// two would otherwise disagree about the very interaction one of them set up.
    ///
    /// The counter itself is the answer when there is nobody behind it — a desk with no one at it
    /// is what the player is facing, and nothing else is in play.
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
        self.route_to_face_within(&dist, &came_from, target, required)
    }

    /// [`Self::route_to_face_dir`] against a search somebody else has already run.
    ///
    /// ⚠️ **A `route_to_face` is a whole Dijkstra, so a caller that asks about every tile of a map
    /// is quadratic in the tile count — and one of them was.** `fishing::nearest_castable_water`
    /// sweeps the map for the nearest water it can cast at, and called `route_to_face` per water
    /// tile; `actions()` calls that, and the route follower re-derives `actions()` **every 20 ms
    /// agent tick**. On Route 23 (369 water tiles) that measured **117 ms per tick against a 20 ms
    /// budget**, and it was Surf that made it visible, because water is a pass-through node only
    /// once the party can mount it, which triples what each of those 369 searches has to explore
    /// (1490 reachable tiles against 551).
    ///
    /// ⚠️ **It does not look like slow motion, it looks like a stutter**, which is why it went
    /// unrecognised as a performance fault. `host.rs` publishes **one video frame per loop
    /// iteration** and each iteration emulates up to `MAX_CATCHUP` of game time, so an iteration
    /// that spends 1.5 s of wall clock on twelve of these ticks advances the game a whole 250 ms —
    /// about one walking step — and shows the viewer nothing in between. Measured on Route 23 at
    /// 20 % speed: **1.9 s of wall clock per tile, worst 4.2 s**, against 0.28 s of game time.
    /// The player jumps a tile, the picture sits still for seconds, the player jumps another tile.
    ///
    /// Sweeping through this instead is one search for the lot: the same map measures **0.37 ms**
    /// and 98 % speed. Anything asking about more than one target wants this.
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
                    MetaTile::Switch { .. } => write!(f, "s")?,
                    MetaTile::CutTree => write!(f, "t")?,
                    // Never in `meta_tiles` either — a push and a cut are actions on the ordinary
                    // floor beside the thing they are about, which is drawn as itself.
                    MetaTile::Boulder { .. } | MetaTile::Cut { .. } => write!(f, "_")?,
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

/// One hidden object the player can press A on, and how the cartridge insists on being approached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HiddenObjectSite {
    /// The tile the player ends up **facing**, which is the one the ROM matches on. It is never
    /// walkable: a hidden object is drawn as the wall or the scenery it hides in.
    pub at: Point8,
    pub object: HiddenObject,
    /// The direction the player must be facing, where the routine behind the object checks. `None`
    /// means any side that can be reached will do.
    ///
    /// ⚠️ **The `SPRITE_FACING_*` argument in `hidden_events.asm` is not this and does not restrict
    /// anything** — `data/events/hidden_events.asm` says so itself, and matching is purely on the
    /// tile in front. It is the *routines* that check, so this column is transcribed from the
    /// handler rather than from the table: `Mansion?Script_Switches` opens
    /// `cp SPRITE_FACING_UP / ret nz`, and `GymTrashScript` and the bg-event signs open with no such
    /// test at all. Getting it wrong is silent — a side approach is dispatched and returns without
    /// drawing anything, so the agent stands there pressing A for ever. That is the same trap
    /// [`pc_locations_for`] carries.
    pub facing: Option<PlayerFacingDirection>,
}

impl HiddenObjectSite {
    const fn new(x: u8, y: u8, object: HiddenObject, facing: Option<PlayerFacingDirection>) -> Self {
        Self { at: Point8 { x, y }, object, facing }
    }
}

/// Every hidden object on `map` that a playthrough has to press, transcribed from pokered's
/// `data/events/hidden_events.asm` and `data/maps/objects/*.asm`.
///
/// ⚠️ **This is a *shortlist*, and the omissions are the design.** The ROM's hidden-object tables
/// also hold slot machines, town signs, gym statues, the Pokédex rating machine and every hidden
/// item in the game. A sign is text the turn already puts on screen; a hidden item is invisible by
/// construction, so a row pointing at one is the game's own secret given away; a slot machine is a
/// menu with no tool behind it, which is the mistake `MetaTile::Pc` spent a release making. What is
/// here is what a run **cannot finish without**:
///
/// - **Vermilion Gym's fifteen bins.** Two hold the switches that open Lt. Surge's door, and which
///   two is re-rolled every time the first is found. No Thunder Badge without them.
/// - **The Celadon Mart roof drink machines.** The Saffron gate guards want a drink and there is
///   nowhere else in the game to buy one. No Saffron, so no Silph Co and no Marsh Badge.
/// - **The Game Corner poster.** It is the only way into the Rocket Hideout, so no Silph Scope, no
///   Pokémon Tower, no Poké Flute and no way past either Snorlax.
/// - **The Pokémon Mansion statues.** They toggle the gates between the Secret Key and the door, so
///   no Volcano Badge.
///
/// ⚠️ **The coordinates are `(x, y)` even though the macro reads `hidden_event Y, X`.** The
/// disassembly's own argument order is the opposite of the one every coordinate in this file uses,
/// and `bg_event 10, 1, …VENDING_MACHINE1` against `DeterministicPolicy`'s proven
/// `UseVendingMachine { at: (10, 1) }` is what pins which way round it goes.
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

/// The floor panel on `map` and the floors its menu lists, in menu order — `None` for the 245 maps
/// that are not a lift.
///
/// Transcribed from each lift's `*ElevatorWarpMaps` table in pokered `scripts/`, which is the list
/// `DisplayElevatorFloorMenu` draws, so the index of a map here **is** the cursor row the driver has
/// to land on.
///
/// ⚠️ **A menu index is not a thing to ask a model for.** `FieldMove::UseElevator` takes one because
/// `DeterministicPolicy` writes routes against a table it can see; a model naming `SilphCo5F` and
/// having it turned into `4` here cannot be off by one, and does not have to be told the order.
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
        // ⚠️ B3F is missing on purpose: the hideout's lift serves three floors and the stairs serve
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

    /// ⚠️ **The bins have to be the ones the puzzle itself numbers, and a transposed `(x, y)` is the
    /// failure this catches.** `hidden_events.asm` writes the pair the other way round from every
    /// coordinate in this file, and both readings land inside Vermilion Gym, so a swap would produce
    /// fifteen rows that route, walk and press A on the wrong tiles for ever.
    /// [`trash_can_position`](crate::pokemon::trash_can_position) is derived independently — it is
    /// what `DeterministicPolicy` turns `wFirstLockTrashCanIndex` into — so agreeing with it is a
    /// real second opinion rather than the same transcription read twice.
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

    /// The rest of the table, against the disassembly. ⚠️ **The facing column is the half that fails
    /// silently**: a statue approached from the side is dispatched and returns without drawing
    /// anything, so the agent stands there pressing A until `DRIVER_ESCAPE_SILENCE`. Every
    /// `Mansion?Script_Switches` opens `cp SPRITE_FACING_UP / ret nz`; the bg-event machines and the
    /// poster open with no such test.
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
        // A Pokémon Centre has a PC and no hidden object, which is what keeps the two tables apart.
        assert!(hidden_objects_for(Map::CeruleanPokecenter).is_empty());
        assert!(hidden_objects_for(Map::PalletTown).is_empty());
    }

    /// The three lifts, and the floors each one's own `*ElevatorWarpMaps` table lists — in order,
    /// because the index **is** the cursor row `DisplayElevatorFloorMenu` lands on.
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

    /// Build a synthetic `MetaTileMap` from ASCII: `#`=wall, `.`=floor, `P`=player, `S`=switch(floor),
    /// `W`=an inter-map warp tile (walkable — the player may stand on it to push), digits `1..9`=boulders,
    /// `=`=a counter (for the reach-over-a-desk test below; the solver never meets one).
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
            player_position: player, player_direction: PlayerFacingDirection::Down,
            map: Map::VictoryRoad1F, width: w, height: h, meta_tiles: meta,
            raw_tile_ids: vec![0; w * h], tileset: crate::pokemon::map_header::TileSetId::Cavern,
            tile_pair_collisions: vec![],
            tile_pair_collisions_water: vec![], sprites,
            warp_targets: HashSet::new(), connection_targets: HashSet::new(),
            spinners: HashMap::new(), can_surf: false, best_rod: None, can_cut: false,
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

    /// ⚠️ **A nurse, a clerk and a receptionist are all talked to *over* something.** The route
    /// `actions()` builds for one stops a tile short and faces the counter, so the plain tile in
    /// front is `Counter` and never the person — which is what reported every conversation in a
    /// Pokémon Centre as "✗ gave up on Nurse: it was interrupted".
    #[test]
    fn an_interaction_reaches_over_a_counter() {
        // Player at (2,3), counter at (2,2), the person behind it at (2,1).
        let (mut map, _) = from_ascii(&["#####", "#.1.#", "#.=.#", "#.P.#", "#####"]);
        // `from_ascii` leaves a sprite's own cell walkable, because the boulder solver moves
        // sprites about and reads them from `sprites`. A map built from the ROM overlays them
        // (`MapMetadata::meta_tiles`), which is the grid this answers off.
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
        // Boulder walled so it can only wobble left/right in a 1-wide slot, never reaching the switch.
        let (map, switch) = from_ascii(&["#######", "#.....#", "#.###.#", "#.#1#.#", "#.#.#.#", "#..P.S#", "#######"]);
        // The boulder at (3,3) sits in a vertical dead-end; the switch (5,5) is unreachable for it.
        assert!(map.solve_boulder_push(switch).is_none());
    }
}
