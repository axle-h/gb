//! What a Pokémon is wherever it is kept (`box_struct`, `party_struct`), with its names beside it,
//! and the Pokédex's two flag arrays.

use poke_core::base_stats::BaseStats;
use poke_core::move_name::PokemonMoveName;
use poke_core::species::PokemonSpecies;
use serde::{Deserialize, Serialize};
use crate::systems::math::{flag_action, FlagAction};
use crate::systems::stats::Dvs;

pub const PARTY_LENGTH: usize = 6;
pub const MONS_PER_BOX: usize = 20;
pub const NUM_MOVES: usize = 4;
/// HP, attack, defense, speed, special: `CalcStat`'s order.
pub const NUM_STATS: usize = 5;

/// `box_struct`: a Pokémon in a box, at the day care, or the part of a party mon that goes with it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoxMon {
    pub species: PokemonSpecies,
    pub hp: u16,
    /// `MON_BOX_LEVEL`: the level when it was last put away. Nothing recomputes it but a deposit.
    pub box_level: u8,
    pub status: u8,
    pub types: [u8; 2],
    /// `MON_CATCH_RATE`: the species' catch rate when it was made, never updated.
    pub catch_rate: u8,
    pub moves: [Option<PokemonMoveName>; NUM_MOVES],
    pub ot_id: u16,
    /// 24 bits.
    pub exp: u32,
    pub stat_exp: [u16; NUM_STATS],
    pub dvs: Dvs,
    /// PP left in the low six bits, PP Ups in the top two.
    pub pp: [u8; NUM_MOVES],
}

/// `party_struct`: a box mon with its level and stats worked out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartyMon {
    pub mon: BoxMon,
    pub level: u8,
    /// Max HP first.
    pub stats: [u16; NUM_STATS],
}

/// A mon with its OT and nickname, as charmap bytes, unterminated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Named<M> {
    pub mon: M,
    pub ot: Vec<u8>,
    pub nick: Vec<u8>,
}

impl BoxMon {
    pub fn base_stats(&self) -> BaseStats {
        BaseStats::of(self.species)
    }
}

/// `wPokedexOwned` and `wPokedexSeen`, one bit per dex number from 1, least significant first.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pokedex {
    pub owned: [u8; 19],
    pub seen: [u8; 19],
}

impl Pokedex {
    pub fn is_owned(&self, species: PokemonSpecies) -> bool {
        let mut owned = self.owned;
        flag_action(&mut owned, Self::bit(species), FlagAction::Test) != 0
    }

    pub fn is_seen(&self, species: PokemonSpecies) -> bool {
        let mut seen = self.seen;
        flag_action(&mut seen, Self::bit(species), FlagAction::Test) != 0
    }

    /// Owned is seen as well, which every caller that sets one sets both for.
    pub fn set_owned(&mut self, species: PokemonSpecies) {
        flag_action(&mut self.owned, Self::bit(species), FlagAction::Set);
        flag_action(&mut self.seen, Self::bit(species), FlagAction::Set);
    }

    /// `IndexToPokedex`, less one.
    fn bit(species: PokemonSpecies) -> u8 {
        BaseStats::of(species).dex - 1
    }
}
