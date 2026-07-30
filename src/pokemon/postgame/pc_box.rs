//! Workstream **A — Pokémon storage (PC boxes)**. See `docs/postgame-coverage-plan.md` §6-A.
//!
//! Deposit / withdraw / release / change box, via Bill's PC inside the parent Pokémon Center PC menu.
//! Unblocks holding more than six Pokémon, which C, D, E, F and G all want.
//!
//! Sub-steps: A1 read box state · A2 open Bill's PC · A3 deposit · A4 withdraw · A5 change box ·
//! A6 release · A7 round-trip test + `postgame-pc-box.bin`.

use crate::pokemon::agent::PokemonAgent;
use crate::pokemon::PokemonApi;

/// What a box operation is doing. Reserved seam (task 0.8) — carried in `AgentState::UsingPcBox`.
/// **Draft shape**: workstream A should reshape this freely once the real menu chain is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PcBoxOp {
    /// Party slot → box.
    Deposit { slot: u8 },
    /// Box slot → party.
    Withdraw { box_slot: u8 },
    /// Switch to box `n` (0-based). ⚠️ This **saves the game**: expect a confirmation and a pause.
    ChangeBox { n: u8 },
    /// Permanently release the box member at `box_slot`.
    Release { box_slot: u8 },
}

/// Reserved seam (task 0.8) — the Bill's-PC box-menu driver for `AgentState::UsingPcBox`.
///
/// The two menus above it are already measured (Phase 0 task 0.4, and the §11 entry for it):
/// the parent PC menu puts `BILL's PC` at index **0** — unconditionally, since the entries that vary
/// all come later — and its submenu reads
/// `WITHDRAW Pokémon / DEPOSIT Pokémon / RELEASE Pokémon / CHANGE BOX / SEE YA!`, indices 0–4.
/// [`crate::pokemon::postgame::item_storage`] drives the sibling item PC and is the closest model.
pub fn tick(_agent: &mut PokemonAgent, _api: &mut PokemonApi<'_>, _op: PcBoxOp) -> Result<(), String> {
    todo!("workstream A — Pokémon storage (PC boxes); see docs/postgame-coverage-plan.md §6-A")
}
