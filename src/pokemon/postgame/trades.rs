//! Workstream **G-trades — the in-game trade driver**. See `docs/postgame-coverage-plan.md` §6-G
//! (sub-steps G5–G6), including the table of the nine usable trades.
//!
//! ⚠️ Each trade requires **already owning the give-species**, so this is not the cheap dex win it
//! looks like. `Route18Gate2F` (Slowbro → Lickitung) additionally needs the Bicycle, so that one row
//! depends on workstream **B**.
//!
//! Sub-steps: G5 the trade driver (`PolicyStep::TradePokemon { give_slot, at }`) · G6 three more
//! trades.

use crate::pokemon::agent::PokemonAgent;
use crate::pokemon::map::Map;
use crate::pokemon::PokemonApi;

/// Reserved seam (task 0.8) — the in-game trade driver (G5).
pub fn tick(_agent: &mut PokemonAgent, _api: &mut PokemonApi<'_>, _give_slot: u8, _at: Map) -> Result<(), String> {
    todo!("workstream G-trades — the trade driver; see docs/postgame-coverage-plan.md §6-G5")
}
