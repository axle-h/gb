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
use crate::pokemon::item::ItemId;
use crate::pokemon::map::{Map, MapSprite};
use crate::pokemon::policy::PolicyStep;
use crate::pokemon::PokemonApi;

/// Reserved seam (task 0.8) — hidden-item search (H4).
pub fn tick(_agent: &mut PokemonAgent, _api: &mut PokemonApi<'_>, _at: Point8) -> Result<(), String> {
    todo!("workstream H — hidden items; see docs/postgame-coverage-plan.md §6-H4")
}

impl PolicyStep {
    /// **H1/H2** — collect **HM05 Flash** from the Route 2 Gate aide, then teach it and light a cave.
    ///
    /// The aide's gate is a plain `hOaksAideRequirement = 10` against **dex owned**
    /// (`scripts/Route2Gate.asm:9-27`), so nothing here is conditional beyond having got there — but
    /// three preconditions are, and each would fail quietly:
    ///
    /// 1. **A free bag slot.** `OaksAideScript` refuses with "you have no room for this item" and the
    ///    conversation ends normally, so a full bag reads exactly like a successful one.
    /// 2. **A party member that can learn Flash.** Only **Slowpoke** and **Mr. Mime** can, of
    ///    everything this save has ever owned — Venusaur, Articuno, Vaporeon, Lapras, Aerodactyl,
    ///    Hitmonlee, Omanyte, Tangela and Farfetch'd all cannot. `flash_slot` is where the caller has
    ///    put one; `withdraw` names it if it has to come out of the box first.
    /// 3. **Route 2's gate is on its north half.** `Route2Gate`'s doors are Route 2 (16,35) and
    ///    (15,39), and the north strip only reaches them past the cut tree at (5,10) —
    ///    `postgame::trades` has the map.
    pub fn flash_steps(withdraw: Option<u8>, shed: ItemId, flash_slot: u8) -> Vec<Self> {
        let mut s = vec![Self::Fly { to: Map::ViridianCity }, Self::enter(Map::ViridianPokecenter)];
        // Free a bag slot for the HM, and fetch the only mon that can hold it.
        s.push(Self::deposit_item(shed, u8::MAX, Map::ViridianPokecenter));
        s.extend(withdraw.map(|box_slot| Self::withdraw_pokemon(box_slot, Map::ViridianPokecenter)));
        s.extend([
            Self::enter(Map::ViridianCity),
            Self::Fly { to: Map::PewterCity },
            Self::enter(Map::Route2),
            Self::CutTree { map: Map::Route2 },
            Self::enter(Map::Route2Gate),
        ]);
        s.extend(std::iter::repeat_n(Self::Interact(MapSprite::ROUTE2GATE_OAKS_AIDE), 3));
        s.extend([
            Self::enter(Map::Route2),
            Self::TeachMove { item: ItemId::Hm05Flash, target_slot: flash_slot },
            // …and prove it. Rock Tunnel is the game's one genuinely dark map: entering 1F sets
            // `wMapPalOffset = 6` and only Flash clears it.
            Self::Fly { to: Map::LavenderTown },
            Self::enter(Map::Route10),
            Self::enter(Map::RockTunnel1F),
            Self::UseFlash { slot: flash_slot },
        ]);
        s
    }
}
