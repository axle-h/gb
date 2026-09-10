//! Tests for the postgame coverage workstreams — see `docs/postgame-coverage-plan.md`.
//!
//! One file per workstream, owned by that workstream and by nothing else (§4.1). All of these are
//! `slow-tests` tier unless they emulate under ~30 s of game time; wall clock ≈ emulated-minutes ÷ 28,
//! so say the budget in the test's doc comment.
//!
//! One exception, and it is the reason the `very-slow-tests` feature exists: a leg that emulates more
//! game time than the rest of `slow-tests` put together belongs in its own tier rather than quietly
//! becoming that tier's wall clock. See `safari::can_sweep_the_safari_zone`.
//!
//! Every workstream roots at `post-hall-of-fame.bin` and commits `postgame-<stream>.bin`. They are
//! **siblings off that root, not a chain** — unlike the main leg tests in the parent module, a failure
//! in one cannot invalidate another's input.

mod phase0;
mod pc_box;
mod fly_bike;
mod fishing;
mod legendaries;
mod safari;
mod game_corner;
mod gifts;
mod trades;
mod aides;
mod items;
mod maps;
