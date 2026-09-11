//! `engine/pokemon/add_mon.asm`: a new mon for a party, and a mon moved between the party and a box
//! or the day care. The Cable Club's `_AddEnemyMonToPlayerParty` is left out.

use poke_core::base_stats::BaseStats;
use poke_core::move_name::PokemonMoveName;
use poke_core::moves::MoveData;
use poke_core::species::PokemonSpecies;
use serde::{Deserialize, Serialize};
use crate::party::{BoxMon, Named, PartyMon, Pokedex, NUM_STATS, PARTY_LENGTH};
use crate::rng::Rng;
use super::evos_moves::{write_mon_moves, Learner};
use super::experience::{calc_experience, calc_level_from_experience};
use super::stats::{calc_stat, calc_stats, Dvs, Stat};

/// `ATKDEFDV_TRAINER`, `SPDSPCDV_TRAINER`.
pub const TRAINER_DVS: Dvs = Dvs([0x98, 0x88]);

/// Where `_AddPartyMon` takes a new mon's DVs, HP, status and stats from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Origin {
    /// The player's, outside a battle: random DVs, full HP.
    Given,
    /// The player's, from a wild battle: the enemy battle mon's.
    Caught { dvs: Dvs, hp: u16, status: u8, stats: [u16; NUM_STATS] },
    /// A trainer's: the fixed trainer DVs, full HP.
    Trainer,
}

/// `_AddPartyMon`'s mon, before it has a party or a name. `Given` takes two bytes of the RNG:
/// speed and special first.
pub fn new_party_mon(species: PokemonSpecies, level: u8, ot_id: u16, origin: &Origin, rng: &mut impl Rng) -> PartyMon {
    let base = BaseStats::of(species);
    let full_hp = |dvs| calc_stat(Stat::Hp, base.stats[0], dvs, None, level);
    let (dvs, hp, status) = match *origin {
        Origin::Given => {
            let speed_special = rng.random();
            let dvs = Dvs([rng.random(), speed_special]);
            (dvs, full_hp(dvs), 0)
        }
        Origin::Caught { dvs, hp, status, .. } => (dvs, hp, status),
        Origin::Trainer => (TRAINER_DVS, full_hp(TRAINER_DVS), 0),
    };
    let mut moves = base.level_1_moves.map(PokemonMoveName::from_repr);
    let mut pp = [0; 4];
    write_mon_moves(species, level, &mut moves, &mut pp, Learner::New);
    let stats = match *origin {
        Origin::Caught { stats, .. } => stats,
        _ => calc_stats(base.stats, dvs, None, level),
    };
    PartyMon {
        mon: BoxMon {
            species,
            hp,
            box_level: 0,
            status,
            types: base.types,
            catch_rate: base.catch_rate,
            moves,
            ot_id,
            exp: calc_experience(base.growth_rate, level),
            stat_exp: [0; NUM_STATS],
            dvs,
            pp: moves.map(|learnt| learnt.map_or(0, |learnt| MoveData::of_move(learnt).pp)),
        },
        level,
        stats,
    }
}

/// `_AddPartyMon`'s bookkeeping for the player: refused when the party is full, and otherwise the
/// species marked owned and seen. The OT is the player's name, for a trainer's party too.
pub fn add_party_mon(party: &mut Vec<Named<PartyMon>>, named: Named<PartyMon>, pokedex: Option<&mut Pokedex>) -> bool {
    if party.len() >= PARTY_LENGTH {
        return false;
    }
    if let Some(pokedex) = pokedex {
        pokedex.set_owned(named.mon.mon.species);
    }
    party.push(named);
    true
}

/// `_MoveMon` into the party, from a box or the day care: the level worked out from experience and
/// the stats afresh with stat experience. HP and status come as they were.
pub fn withdraw(mon: BoxMon) -> PartyMon {
    let base = mon.base_stats();
    let level = calc_level_from_experience(base.growth_rate, mon.exp);
    let stats = calc_stats(base.stats, mon.dvs, Some(mon.stat_exp), level);
    PartyMon { mon, level, stats }
}

/// `_MoveMon` out of the party, into a box or the day care: the level it leaves at is its box level.
pub fn deposit(party_mon: PartyMon) -> BoxMon {
    BoxMon { box_level: party_mon.level, ..party_mon.mon }
}

#[cfg(test)]
mod tests {
    use crate::fixtures::cases;
    use crate::rng::GameRng;
    use super::*;

    #[derive(Deserialize)]
    struct NewPartyMonInput {
        species: PokemonSpecies,
        level: u8,
        ot_id: u16,
        origin: Origin,
    }

    #[test]
    fn every_harvested_case_of_add_party_mon() {
        for (i, output, rng) in cases::<NewPartyMonInput, PartyMon>(include_str!("../../fixtures/pokemon/add_party_mon.jsonl")) {
            let mut tape = GameRng::tape(rng);
            assert_eq!(new_party_mon(i.species, i.level, i.ot_id, &i.origin, &mut tape), output,
                "{} at {}, {:?}", i.species, i.level, i.origin);
        }
    }

    #[test]
    fn every_harvested_case_of_withdraw() {
        for (input, output, _) in cases::<BoxMon, PartyMon>(include_str!("../../fixtures/pokemon/withdraw.jsonl")) {
            assert_eq!(withdraw(input.clone()), output, "{input:?}");
        }
    }

    #[test]
    fn every_harvested_case_of_deposit() {
        for (input, output, _) in cases::<PartyMon, BoxMon>(include_str!("../../fixtures/pokemon/deposit.jsonl")) {
            assert_eq!(deposit(input.clone()), output, "{input:?}");
        }
    }

    #[test]
    fn a_full_party_refuses_and_leaves_the_pokedex_alone() {
        let mut pokedex = Pokedex::default();
        let mon = new_party_mon(PokemonSpecies::Pidgey, 3, 0, &Origin::Trainer, &mut GameRng::tape(vec![]));
        let named = Named { mon, ot: vec![], nick: vec![] };
        let mut party = vec![];
        for _ in 0..PARTY_LENGTH {
            assert!(add_party_mon(&mut party, named.clone(), Some(&mut pokedex)));
        }
        let mut other = named.clone();
        other.mon.mon.species = PokemonSpecies::Rattata;
        assert!(!add_party_mon(&mut party, other, Some(&mut pokedex)));
        assert!(pokedex.is_owned(PokemonSpecies::Pidgey) && !pokedex.is_seen(PokemonSpecies::Rattata));
    }
}
