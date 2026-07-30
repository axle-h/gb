//! Workstream **H — Oak's aides**. See `docs/postgame-coverage-plan.md` §6-H. Depends on A–G for dex
//! count.
//!
//! Three items gated on **dex owned**: HM05 Flash at 10 (Route 2 Gate), the Itemfinder at 30
//! (`Route11Gate2F`), Exp.All at 50 (`Route15Gate2F`). Check the gate with `probe_coverage` before
//! travelling — don't guess.
//!
//! Sub-steps: H1 Flash · H2 teach Flash and prove it · H3 Itemfinder · H4 hidden items
//! (`PolicyStep::SearchHiddenItem { at }` — bg-event objects, same shape as the `FlipSwitch` tiles) ·
//! H5 Exp.All, then `postgame-aides.bin`.

use crate::geometry::Point8;
use crate::pokemon::agent::PokemonAgent;
use crate::pokemon::PokemonApi;

/// Reserved seam (task 0.8) — hidden-item search (H4).
pub fn tick(_agent: &mut PokemonAgent, _api: &mut PokemonApi<'_>, _at: Point8) -> Result<(), String> {
    todo!("workstream H — hidden items; see docs/postgame-coverage-plan.md §6-H4")
}
