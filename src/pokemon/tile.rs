use crate::geometry::Point8;
use crate::pokemon::map::Map;

#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord, strum_macros::Display, Default)]
pub enum MetaTile {
    #[default]
    Empty,
    Obstacle,
    /// Water tile (tile ID 0x14) — can only be crossed while surfing.
    Water,
    /// Ledge tile — can only be crossed by jumping in the specified direction.
    Jump(JumpDirection),
    Sprite(&'static str),
    Warp { to_map: Map, to_position: Point8 },
    /// Walkable entry point into an adjacent map.
    Connection { to_map: Map, to_position: Point8 },
    /// Water entry point into an adjacent map — only reachable while surfing.
    ConnectionWater(Map),
    /// Counter / desk tile listed in `wTilesetTalkingOverTiles`.
    /// The player cannot walk on it, but can interact with a sprite one tile behind it
    /// by facing the counter and pressing A — pokered's "talking over" mechanic.
    Counter,
    /// A shrub that blocks passage until the player uses HM Cut.
    /// Treated as impassable until `can_use_cut` is true.
    CutTree,
    /// A PC (a hidden-object tile the player faces and presses A to use — Someone's PC / Bill's PC).
    /// Impassable like `Obstacle`, but `actions()` emits a route that faces it and presses A. The
    /// tile is not classified from the tileset; PC coordinates are looked up per map (`pc_locations`).
    Pc,
    /// Tall-grass tile (tile ID matches `wGrassTile` for the current tileset).
    /// Walkable; stepping on it can trigger a wild Pokémon encounter.
    Grass,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub struct WarpEvent {
    pub position: Point8,
    pub destination_map: Map,
    pub destination_position: Point8,
}

impl WarpEvent {
    pub fn tile(&self) -> MetaTile {
        MetaTile::Warp {
            to_map: self.destination_map,
            to_position: self.destination_position,
        }
    }
}

/// The direction a ledge can be jumped over.
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord, strum_macros::Display)]
pub enum JumpDirection {
    South,
    West,
    East,
}