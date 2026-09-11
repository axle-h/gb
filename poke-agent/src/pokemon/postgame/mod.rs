//! The post-game mechanisms, each module adding `PolicyStep` constructors such as `old_rod_steps()`.

/// The debug tier, the one place RAM writes are allowed.
pub mod debug;

pub mod item_storage;

pub mod pc_box;
pub mod fly_bike;
pub mod fishing;
pub mod legendaries;
pub mod safari;
pub mod game_corner;
pub mod gifts;
pub mod trades;
pub mod aides;
pub mod items;
pub mod maps;
