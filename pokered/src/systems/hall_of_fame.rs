//! `HoFRecordMonInfo` and `SaveHallOfFameTeams`: the team a win records, and the record of every
//! team that has won.

use poke_core::species::PokemonSpecies;
use serde::{Deserialize, Serialize};

/// `HOF_TEAM_CAPACITY`: the record keeps this many teams and drops the oldest past it.
pub const HOF_TEAM_CAPACITY: usize = 50;

/// `HOF_MON`: one entry of `wHallOfFame`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HallOfFameMon {
    pub species: PokemonSpecies,
    pub level: u8,
    /// Charmap bytes, unterminated.
    pub nick: Vec<u8>,
}
