pub mod colour;
pub mod compose;
pub mod layers;
pub mod tiles;
pub mod ui;

use serde::{Deserialize, Serialize};
use layers::{Effects, MapLayer, Object};
use tiles::TileData;
use ui::UiSurface;

/// Everything the compositor reads, which the game writes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Screen {
    pub tiles: TileData,
    pub ui: UiSurface,
    pub map: MapLayer,
    /// OAM: forty objects at most.
    pub sprites: Vec<Object>,
    pub effects: Effects,
}
