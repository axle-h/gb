pub mod colour;
pub mod compose;
pub mod layers;
pub mod mon_icons;
pub mod sgb;
pub mod text_boxes;
pub mod tiles;
pub mod ui;

use serde::{Deserialize, Serialize};
use layers::{Effects, MapLayer, Object, TileMap, Window};
use sgb::SgbState;
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
    /// The SGB's palettes and per-cell attributes, which only the SGB colour modes read.
    pub sgb: SgbState,
    /// When set, the background is this map under the scroll rather than the UI surface over the map.
    #[serde(default)]
    pub background: Option<TileMap>,
    /// When set, the window, from `x - 7` across and `y` down to the screen's bottom right.
    #[serde(default)]
    pub window: Option<Window>,
}
