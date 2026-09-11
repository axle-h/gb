//! ```text
//! impl PolicyStep {
//!     pub fn fishing_steps() -> Vec<PolicyStep> { … }
//! }
//! ```

/// Phase 0 infrastructure, not workstreams — the debug tier (0.7) and item PC storage (0.5/0.6).
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
