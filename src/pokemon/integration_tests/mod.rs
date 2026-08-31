//! End-to-end tests that boot the emulator and drive the real agent.
//!
//! These are tiered by how much **game time** they emulate, because that is what they cost: the
//! emulator core runs at ~34× realtime and the agent costs ~16% on top, giving **~28×** end to end
//! (measured 2026-08-05 by `game_boy::tests::bench_core_throughput` and
//! [`fixture::bench_emulation_throughput`]), so wall clock ≈ emulated-minutes ÷ 28.
//!
//! | Tier | How to run | Contents |
//! |---|---|---|
//! | default | `cargo test --release` | [`mechanics`] + the two navigation smoke tests in [`early_game`] |
//! | leg chain | `cargo test --release --features slow-tests` | one test per `PolicyStep::*_steps()` leg, each seeded from a committed snapshot |
//! | full game | `cargo test --release --features full-playthrough` | [`playthrough`] — a fresh save played to all 8 badges + Victory Road 2F. **Run it before pushing** |
//!
//! The leg tests form a linear pipeline: each consumes the snapshot the previous one produces, and
//! together they cover the same ground as [`playthrough`] but in parallel and from disk. Those
//! snapshots live in `src/pokemon/data/*.bin` and are **committed inputs** — a test only rewrites one
//! when `--features regen-fixtures` is on (see [`fixture::TestFixture::save_state_named`]), so an
//! ordinary run leaves the working tree clean.
//!
//! ⚠️ **Keep the chain's order matching `complete_game_steps`' `extend` order.** When they disagree,
//! the leg fixtures quietly encode the *older* order and the failure surfaces somewhere else entirely
//! — as an unwinnable fight, or as a mon in the wrong party slot. Two tests sat `#[ignore]`d on that:
//! `can_get_silph_card_key` was seeded from `at-saffron.bin` while the mainline fetches Vaporeon first,
//! so `can_beat_silph_giovanni` met the 7F rival with no Vaporeon. The endgame chain still has the same
//! divergence — `post-volcano-lone.bin` predates `seafoam_articuno_steps` — which is why
//! [`endgame::can_solve_victory_road_1f`] seeds its Articuno rather than earning it.
//!
//! # Naming party members
//!
//! Steps that act on a particular **mon** — `TeachMove`, `EvolveWithStone`, `UseStrength` — take a
//! [`crate::pokemon::policy::PartyRef`] and should name it by `Species`, not by `Slot`. A slot index is
//! a guess about how many members the party happened to hold when the run reached that step, and a
//! fixture does not have to carry the same party the mainline does. Three tests sat `#[ignore]`d on
//! that for a long time, none of them naming it: two blamed a stale fixture and one blamed deep-bag
//! menu scrolling.
//!
//! ⚠️ **`Slot` is almost never right, and the one exception it used to have has gone.** "Cut always
//! asks slot 0" was true only while the starter carried Cut; `policy::field_move_carrier` resolves
//! the holder out of the live party now, so a field move needs no `PartyRef` at all.
//!
//! ⚠️ **The chain has a *root*, and swapping the starter invalidates all of it at once.**
//! `at-cerulean.bin` is what everything below descends from, and `early_game::regen_at_cerulean_fixture`
//! is what produces it — before that it was `post-cascade.bin`, which no test built, so a change to the
//! mainline party left every downstream leg resolving against a party that no longer existed.

use std::time::Duration;
use crate::cycles::MachineCycles;
use crate::pokemon::policy::{DeterministicPolicy, PartyRef, PolicyStep};
use crate::pokemon::*;
use crate::pokemon::agent::{AgentEvent, OverworldActionAbortedReason, PokemonAgent, AGENT_RESOLUTION,
                            MANUAL_INPUT_CAPACITY, MANUAL_INPUT_TICKS_PER_PRESS};
use crate::pokemon::battle::BattleType;
use crate::pokemon::tile::JumpDirection;
use crate::pokemon::tile::MetaTile;
use crate::pokemon::map::MapSprite;
use crate::ram::{RAM, ROM};

pub(crate) mod fixture;
pub use fixture::TestFixture;

mod mechanics;
mod early_game;
/// Turns abandoned mid-flight because the game asked a different question. See the module header.
mod interruption;
/// **W4** — the LLM path end to end against a mock OpenAI server. Default tier: it is the test that
/// keeps the tool schema, the resolution logic and the agent's expectations in step, and it emulates
/// a few seconds of game time.
#[cfg(feature = "llm")]
mod llm;
mod vermilion;
mod celadon;
mod fuchsia;
mod saffron;
mod cinnabar;
mod endgame;
mod playthrough;
// Default tier: one case per jam `soak` has found, replayed from the save state it was found in.
// Cheap, because each starts inside the jam instead of wandering into one. See the module docs.
mod stalls;
// Gated as a module: without the feature the test does not exist, so it never shows up as ignored.
#[cfg(feature = "soak-tests")]
mod soak;
mod postgame;

pub const PALLET_TOWN_STATE: &[u8] = include_bytes!("../data/pallet-town-state.bin");
pub const ROUTE1_STATE: &[u8] = include_bytes!("../data/route1-state.bin");
pub const BATTLE_STATE: &[u8] = include_bytes!("../data/battle-state.bin");
/// Standing inside the Route 22 gate with no badges, which is where the deployed run of 2026-08-26
/// spent its last twelve turns. The guard turns the player back with a message and a scripted step
/// backwards, which is the shape every blocker in the game has — see
/// `mechanics::a_guard_who_turns_you_back_is_quoted_rather_than_swallowed`.
pub const ROUTE22_GATE: &[u8] = include_bytes!("../data/route22-gate.bin");
