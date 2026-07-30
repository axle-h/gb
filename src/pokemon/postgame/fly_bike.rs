//! Workstream **B — Fly, the Bicycle, and Cycling Road**. See `docs/postgame-coverage-plan.md` §6-B.
//!
//! The biggest quality-of-life win in the plan: Fly collapses cross-Kanto travel, which every other
//! workstream otherwise pays for in emulated minutes.
//!
//! Sub-steps: B1 Bike Voucher · B2 Bicycle · B3 HM02 · B4 teach Fly · B5 the Fly driver (the town map
//! is a bespoke screen, not a `HandleMenuInput` list) · B6 Cycling Road · B7 Route 16 Snorlax, then
//! `postgame-fly-bike.bin`.

use crate::pokemon::agent::PokemonAgent;
use crate::pokemon::map::Map;
use crate::pokemon::PokemonApi;

/// Reserved seam (task 0.8) — the Fly driver for `AgentState::Flying` (B5).
/// START → POKéMON → mon → FLY → town-map cursor.
pub fn tick(_agent: &mut PokemonAgent, _api: &mut PokemonApi<'_>, _to: Map) -> Result<(), String> {
    todo!("workstream B — the Fly driver; see docs/postgame-coverage-plan.md §6-B5")
}
