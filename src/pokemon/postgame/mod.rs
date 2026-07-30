//! Postgame coverage workstreams — see `docs/postgame-coverage-plan.md`.
//!
//! Everything the game offers that the main-quest playthrough never touches: storage, Fly, fishing,
//! the remaining legendaries, the Safari Zone, the Game Corner, gifts and trades, and the
//! dex-gated Oak's-aide items.
//!
//! # Why this module exists at all
//!
//! `policy.rs` and `agent.rs` are three-thousand-line files that several agents need to extend at
//! once. Rust lets an `impl` block for a type live in **any** module of the same crate, so a
//! workstream puts its `PolicyStep` constructors and its driver here —
//!
//! ```ignore
//! impl PolicyStep {
//!     pub fn fishing_steps() -> Vec<PolicyStep> { … }
//! }
//! ```
//!
//! — and touches the shared files on **four lines only**: one `PolicyStep` variant, one `AgentState`
//! variant, and one match arm each, every arm delegating in a single line:
//!
//! ```ignore
//! // policy.rs
//! PolicyStep::Fish { .. } => return postgame::fishing::pick(self, state, world_graph),
//! // agent.rs
//! AgentState::Fishing(s) => return postgame::fishing::tick(self, api, s),
//! ```
//!
//! One-line arms merge cleanly; inline bodies do not. That is the whole point of the seam — see §4.1
//! of the plan before adding anything to a shared file.

/// Landing point for the reserved `PolicyStep` seams (task 0.8) that no workstream owns yet.
///
/// The variants exist so that taking a workstream is a **one-line** edit to `policy.rs` — move your
/// variant out of the grouped arm into its own arm delegating to your module — rather than an enum
/// change plus an arm, which is what makes two agents collide in the same file.
///
/// Unreachable in practice: nothing constructs those variants, so no legitimate run can arrive here.
pub fn unimplemented_seam(step: &crate::pokemon::policy::PolicyStep) -> Option<crate::pokemon::actions::OverworldAction> {
    todo!("reserved postgame seam, not implemented yet: {step:?} — see docs/postgame-coverage-plan.md §6")
}

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
