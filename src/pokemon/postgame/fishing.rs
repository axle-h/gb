//! Workstream **C — Fishing**. See `docs/postgame-coverage-plan.md` §6-C.
//!
//! Opens the whole water encounter table — Magikarp, Goldeen, Poliwag, Tentacool, Krabby, Horsea,
//! Staryu.
//!
//! Sub-steps: C1 Old Rod · C2 the fishing driver (face water, use the rod from the bag, handle the
//! "not even a nibble" / "hooked" text, drop into the normal battle handler on a bite) · C3 catch
//! from a bite · C4 Good Rod · C5 Super Rod, then `postgame-fishing.bin`.

use crate::geometry::Point8;
use crate::pokemon::agent::PokemonAgent;
use crate::pokemon::item::ItemId;
use crate::pokemon::PokemonApi;

/// The three rods, in the order they are obtained. Reserved seam (task 0.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rod { Old, Good, Super }

impl Rod {
    /// The bag item to use from the ITEM menu.
    pub fn item(self) -> ItemId {
        match self { Rod::Old => ItemId::OldRod, Rod::Good => ItemId::GoodRod, Rod::Super => ItemId::SuperRod }
    }
}

/// Reserved seam (task 0.8) — the fishing driver for `AgentState::Fishing` (C2).
pub fn tick(_agent: &mut PokemonAgent, _api: &mut PokemonApi<'_>, _rod: Rod, _at: Point8) -> Result<(), String> {
    todo!("workstream C — the fishing driver; see docs/postgame-coverage-plan.md §6-C2")
}
