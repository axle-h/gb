//! End-to-end tests that boot the emulator and drive the real agent.

use std::time::Duration;
use gb::cycles::MachineCycles;
use crate::pokemon::policy::{DeterministicPolicy, PartyRef, PolicyStep};
use crate::pokemon::*;
use crate::pokemon::agent::{AgentEvent, OverworldActionAbortedReason, PokemonAgent, AGENT_RESOLUTION,
                            MANUAL_INPUT_CAPACITY, MANUAL_INPUT_TICKS_PER_PRESS};
use crate::pokemon::battle::BattleType;
use crate::pokemon::tile::JumpDirection;
use crate::pokemon::tile::MetaTile;
use crate::pokemon::map::MapSprite;
use gb::ram::{RAM, ROM};

/// Each named cell as two tests, one per battle-animation setting: animations stretch every gap a
/// battle bound measures, and only a served run plays with them on.
macro_rules! in_both_animation_modes {
    ($($cell:ident),* $(,)?) => {
        mod animated {
            $( #[test] fn $cell() { super::$cell(crate::pokemon::options::SERVED_OPTIONS) } )*
        }
        mod still {
            $( #[test] fn $cell() { super::$cell(crate::pokemon::options::HEADLESS_OPTIONS) } )*
        }
    };
}

pub(crate) mod fixture;
pub use fixture::TestFixture;

pub(crate) mod cheats;

pub(crate) mod coverage;

pub(crate) mod completion;
#[cfg(feature = "slow-tests")]
pub(crate) mod completion_run;

pub(crate) mod godmode;

mod mechanics;
mod early_game;
/// Turns abandoned mid-flight because the game asked a different question.
mod interruption;
/// The LLM path end to end against a mock OpenAI server.
mod llm;
pub(crate) mod llm_harness;
mod battle_refusals;
mod model_surface;
mod branch_points;
mod vermilion;
mod celadon;
mod fuchsia;
mod saffron;
mod cinnabar;
mod endgame;
mod playthrough;
mod stalls;
// Gated as a module, so without the feature it never shows up as ignored.
#[cfg(feature = "slow-tests")]
mod soak;
mod postgame;

pub const PALLET_TOWN_STATE: &[u8] = include_bytes!("../data/pallet-town-state.bin");
pub const ROUTE1_STATE: &[u8] = include_bytes!("../data/route1-state.bin");
pub const BATTLE_STATE: &[u8] = include_bytes!("../data/battle-state.bin");
pub const ROUTE22_GATE: &[u8] = include_bytes!("../data/route22-gate.bin");
