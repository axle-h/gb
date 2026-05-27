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
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.tile {
            MetaTile::Warp { to_map, to_position }       => write!(f, "Warp → {to_map} {to_position}"),
            MetaTile::Connection { to_map, to_position } => write!(f, "Go to {to_map} {to_position}"),
            MetaTile::Sprite(n)     => write!(f, "Talk to {n}"),
            MetaTile::Grass         => write!(f, "Walk in grass"),
            other                   => write!(f, "{other}"),
        }
    }
}
