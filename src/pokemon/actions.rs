use std::fmt::Display;
use crate::geometry::Point8;
use crate::joypad::JoypadButton;
use crate::pokemon::map::Map;
use crate::pokemon::encoding::MetaTile;

#[derive(Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub struct OverworldAction {
    pub map: Map,
    pub origin: Point8,
    pub destination: Point8,
    pub tile: MetaTile,
    pub route: Vec<JoypadButton>,
}

impl Display for OverworldAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let label = match self.tile {
            MetaTile::Warp(m)       => format!("Warp → {m}"),
            MetaTile::Connection(m) => format!("Go to {m}"),
            MetaTile::Sprite(n)     => format!("Talk to {n}"),
            MetaTile::Grass         => "Walk in grass".to_string(),
            other                   => format!("{other}"),
        };
        write!(f, "{label} ({} steps)", self.route.len())
    }
}
