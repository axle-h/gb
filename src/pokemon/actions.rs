use std::fmt::Display;
use crate::geometry::Point8;
use crate::joypad::JoypadButton;
use crate::pokemon::map::Map;
use crate::pokemon::tile::MetaTile;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OverworldAction {
    pub map: Map,
    pub origin: Point8,
    pub destination: Point8,
    pub tile: MetaTile,
    pub route: Vec<JoypadButton>,
}

impl PartialOrd for OverworldAction {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for OverworldAction {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.tile.cmp(&other.tile)
    }
}

impl Display for OverworldAction {
    /// The imperative form — this is a menu entry, one row of what the policy may choose next, so it
    /// leads with the verb where [`MetaTile`]'s own `Display` is a noun phrase.
    ///
    /// ⚠️ **Every arm that has a target names it.** The three that used to fall through to
    /// `{other}` were the vague ones: a surf crossing read as `ConnectionWater` and did not say
    /// which map it led to, and `Pc`/`CutTree` named the tile rather than the thing to do with it.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.tile {
            MetaTile::Warp { to_map, to_position }       => write!(f, "Warp → {to_map} {to_position}"),
            MetaTile::Connection { to_map, to_position } => write!(f, "Go to {to_map} {to_position}"),
            MetaTile::ConnectionWater(to_map)            => write!(f, "Surf to {to_map}"),
            MetaTile::Sprite(n)     => write!(f, "Talk to {n}"),
            MetaTile::Grass         => write!(f, "Walk in grass"),
            MetaTile::Pc            => write!(f, "Use the PC"),
            MetaTile::CutTree       => write!(f, "Cut the tree"),
            MetaTile::Fish { rod }  => write!(f, "Fish with the {}", rod.name()),
            other                   => write!(f, "{other}"),
        }
    }
}
