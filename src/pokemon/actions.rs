use crate::geometry::Point8;
use crate::joypad::JoypadButton;
use crate::pokemon::map::Map;
use crate::pokemon::MetaTile;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OverworldAction {
    pub map: Map,
    pub origin: Point8,
    pub destination: Point8,
    pub tile: MetaTile,
    pub route: Vec<JoypadButton>,
}
