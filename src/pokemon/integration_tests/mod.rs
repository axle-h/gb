//! End-to-end tests that boot the emulator and drive the real agent.
//!
//! These are tiered by how much **game time** they emulate, because that is what they cost: the
//! emulator runs at roughly 23× realtime and the agent adds only ~11% on top of it (see
//! [`fixture::bench_emulation_throughput`]), so wall clock is almost exactly emulated-minutes ÷ 23.
//!
//! | Tier | How to run | Contents |
//! |---|---|---|
//! | default | `cargo test --release` | [`mechanics`] + the two navigation smoke tests in [`early_game`] |
//! | leg chain | `cargo test --release --features slow-tests` | one test per `PolicyStep::*_steps()` leg, each seeded from a committed snapshot |
//! | full game | `cargo test --release --features full-playthrough` | [`playthrough`] — a fresh save played to the Hall of Fame |
//!
//! The leg tests form a linear pipeline: each consumes the snapshot the previous one produces, and
//! together they cover the same ground as [`playthrough`] but in parallel and from disk. Those
//! snapshots live in `src/pokemon/data/*.bin` and are **committed inputs** — a test only rewrites one
//! when `--features regen-fixtures` is on (see [`fixture::TestFixture::save_state_named`]), so an
//! ordinary run leaves the working tree clean.
//!
//! # Known failures
//!
//! Four tests are `#[ignore]`d for a *named* reason rather than because they are slow. Each doc
//! comment carries the diagnosis; the short version:
//!
//! | Test | Cause |
//! |---|---|
//! | [`fuchsia::can_get_poke_flute`] | leg bug — wedges on the Rocket Hideout elevator exit; fails identically before this module was split up |
//! | [`endgame::can_solve_victory_road_1f`] | `TeachMove` never completes for an HM deep in the bag |
//! | [`saffron::can_get_vaporeon`] | `at-saffron.bin` predates the lone-starter plan, so its slot 1 is a Pidgey and the slot-addressed evolve/teach hits the wrong mon |
//! | [`saffron::can_beat_silph_giovanni`] | same stale chain — `silph-card-key.bin` has no Vaporeon for the 7F rival |
//!
//! The two `saffron` ones share a root cause worth stating once: `PolicyStep` addresses party members
//! by **slot**, and every committed fixture from `post-cascade.bin` onward still carries the early
//! Route-1 Pidgey that `complete_game_steps` no longer catches. Regenerating a single snapshot cannot
//! fix that — it needs a fresh [`playthrough::full_playthrough`] to re-cut the chain, or party
//! references moved from slot to species.

use std::time::Duration;
use crate::cycles::MachineCycles;
use crate::pokemon::policy::{DeterministicPolicy, PolicyStep};
use crate::pokemon::*;
use crate::pokemon::agent::{AgentEvent, OverworldActionAbortedReason, PokemonAgent, AGENT_RESOLUTION};
use crate::pokemon::battle::BattleType;
use crate::pokemon::tile::JumpDirection;
use crate::pokemon::tile::MetaTile;
use crate::pokemon::map::MapSprite;
use crate::ram::{RAM, ROM};

mod fixture;
pub use fixture::TestFixture;

mod mechanics;
mod early_game;
mod vermilion;
mod celadon;
mod fuchsia;
mod saffron;
mod cinnabar;
mod endgame;
mod playthrough;

pub const PALLET_TOWN_STATE: &[u8] = include_bytes!("../data/pallet-town-state.bin");
pub const ROUTE1_STATE: &[u8] = include_bytes!("../data/route1-state.bin");
pub const BATTLE_STATE: &[u8] = include_bytes!("../data/battle-state.bin");
