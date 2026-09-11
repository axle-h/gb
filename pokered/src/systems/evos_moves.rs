//! `engine/pokemon/evos_moves.asm`: the moves a mon has for its level, the move a level brings, and
//! evolution. What `EvolutionAfterBattle` prints and animates, and `LearnMove`'s questions, are the
//! caller's; this is what they decide and what they change.

use poke_core::base_stats::BaseStats;
use poke_core::evos_moves::{Evolution, EvosMoves};
use poke_core::move_name::PokemonMoveName;
use poke_core::moves::MoveData;
use poke_core::species::PokemonSpecies;
use serde::{Deserialize, Serialize};
use crate::party::{Named, PartyMon, Pokedex, NUM_MOVES};
use super::stats::calc_stats;

/// `wLearningMovesFromDayCare`, and with it `wDayCareStartLevel`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Learner {
    /// A mon being made: every move up to its level, PP left to the caller.
    New,
    /// A mon leaving the day care: only the moves above the level it went in at, each with its PP.
    DayCare { start_level: u8 },
}

/// `WriteMonMoves`: every learnset move up to `level` that is not known already, into the first
/// empty slot or, with none, the last after the first is shifted out. A day care mon's PP shifts
/// with its moves, and a new move's is the move's full PP with no PP Ups.
pub fn write_mon_moves(species: PokemonSpecies, level: u8, moves: &mut [Option<PokemonMoveName>; NUM_MOVES],
                       pp: &mut [u8; NUM_MOVES], learner: Learner) {
    for (move_level, learnt) in EvosMoves::of(species).learnset {
        if level < move_level {
            break;
        }
        if let Learner::DayCare { start_level } = learner && start_level >= move_level {
            continue;
        }
        if moves.contains(&Some(learnt)) {
            continue;
        }
        let slot = moves.iter().position(Option::is_none).unwrap_or_else(|| {
            moves.rotate_left(1);
            if learner != Learner::New {
                pp.rotate_left(1);
            }
            NUM_MOVES - 1
        });
        moves[slot] = Some(learnt);
        if learner != Learner::New {
            pp[slot] = MoveData::of_move(learnt).pp;
        }
    }
}

/// `LearnMoveFromLevelUp`'s question: the first learnset move at exactly `level`, unless it is
/// known. A second move at the same level is never looked at.
pub fn level_up_move(species: PokemonSpecies, level: u8, moves: &[Option<PokemonMoveName>; NUM_MOVES]) -> Option<PokemonMoveName> {
    let (_, learnt) = EvosMoves::of(species).learnset.into_iter().find(|&(at, _)| at == level)?;
    (!moves.contains(&Some(learnt))).then_some(learnt)
}

/// `EvolutionAfterBattle`'s walk of one mon's entries from `from`, outside a trade: the first entry
/// that fires, and what it evolves into. Trade entries never fire; `wForceEvolution` (an item
/// being used) stops the walk at the first level entry.
///
/// `cur_item` is `wCurItem`, which is `wCurPartySpecies`: whatever the battle left there, and after
/// an evolution the new species, since `LearnMoveFromLevelUp` sets it. An item entry compares
/// against that, which is how a battle evolves a mon by stone without one.
pub fn next_evolution(species: PokemonSpecies, level: u8, cur_item: u8, force: bool, from: usize) -> Option<(usize, PokemonSpecies)> {
    for (at, evolution) in EvosMoves::of(species).evolutions.into_iter().enumerate().skip(from) {
        match evolution {
            Evolution::Trade { .. } => {}
            Evolution::Item { item, min_level, into } => {
                if cur_item == item as u8 && level >= min_level {
                    return Some((at, into));
                }
            }
            Evolution::Level { level: needed, into } => {
                if force {
                    return None;
                }
                if level >= needed {
                    return Some((at, into));
                }
            }
        }
    }
    None
}

/// What an evolution that is not cancelled changes: the name if it was the species', the stats with
/// stat experience, HP raised by what max HP gained (16 bits, wrapping), the types, and the Pokédex.
/// The catch rate stays the old species'. Returns `LearnMoveFromLevelUp`'s move for the new species.
pub fn evolve(named: &mut Named<PartyMon>, into: PokemonSpecies, pokedex: &mut Pokedex) -> Option<PokemonMoveName> {
    let PartyMon { mon, level, stats } = &mut named.mon;
    if named.nick == mon.species.name() {
        named.nick = into.name();
    }
    let base = BaseStats::of(into);
    let evolved = calc_stats(base.stats, mon.dvs, Some(mon.stat_exp), *level);
    mon.hp = mon.hp.wrapping_add(evolved[0].wrapping_sub(stats[0]));
    *stats = evolved;
    mon.species = into;
    mon.types = base.types;
    pokedex.set_owned(into);
    level_up_move(into, *level, &mon.moves)
}

#[cfg(test)]
mod tests {
    use poke_core::item::ItemId;
    use crate::fixtures::cases;
    use super::*;
    use PokemonSpecies::*;

    #[derive(Deserialize)]
    struct WriteMonMovesInput {
        species: PokemonSpecies,
        level: u8,
        moves: [Option<PokemonMoveName>; NUM_MOVES],
        pp: [u8; NUM_MOVES],
        learner: Learner,
    }

    #[test]
    fn every_harvested_case_of_write_mon_moves() {
        let cases = cases::<WriteMonMovesInput, ([Option<PokemonMoveName>; NUM_MOVES], [u8; NUM_MOVES])>(
            include_str!("../../fixtures/pokemon/write_mon_moves.jsonl"));
        for (i, output, _) in cases {
            let (mut moves, mut pp) = (i.moves, i.pp);
            write_mon_moves(i.species, i.level, &mut moves, &mut pp, i.learner);
            assert_eq!((moves, pp), output, "{} at {} from {:?} {:?}, {:?}", i.species, i.level, i.moves, i.pp, i.learner);
        }
    }

    #[test]
    fn a_level_brings_one_move_unless_known() {
        use PokemonMoveName::*;
        assert_eq!(level_up_move(Bulbasaur, 7, &[Some(Tackle), Some(Growl), None, None]), Some(LeechSeed));
        assert_eq!(level_up_move(Bulbasaur, 7, &[Some(LeechSeed), None, None, None]), None);
        assert_eq!(level_up_move(Bulbasaur, 8, &[None; 4]), None);
    }

    #[test]
    fn evolution_walks_its_entries_in_order() {
        assert_eq!(next_evolution(Bulbasaur, 16, 0, false, 0), Some((0, Ivysaur)));
        assert_eq!(next_evolution(Bulbasaur, 15, 0, false, 0), None);
        assert_eq!(next_evolution(Bulbasaur, 16, 0, true, 0), None, "an item stops at a level entry");
        assert_eq!(next_evolution(Kadabra, 100, 0, false, 0), None, "no trade outside the Cable Club");
        let water = ItemId::WaterStone as u8;
        assert_eq!(next_evolution(Eevee, 1, water, true, 0), Some((2, Vaporeon)));
        assert_eq!(next_evolution(Eevee, 1, water, true, 3), None);
        assert_eq!(next_evolution(Eevee, 1, ItemId::MoonStone as u8, true, 0), None);
    }

    #[test]
    fn a_wild_mon_whose_id_is_a_stones_evolves_by_stone() {
        assert_eq!(Growlithe as u8, ItemId::ThunderStone as u8);
        assert_eq!(next_evolution(Pikachu, 5, Growlithe as u8, false, 0), Some((0, Raichu)));
    }

    fn charmander(nick: Vec<u8>) -> Named<PartyMon> {
        use crate::rng::GameRng;
        use crate::systems::add_mon::{new_party_mon, Origin};
        let mon = new_party_mon(Charmander, 16, 1, &Origin::Trainer, &mut GameRng::tape(vec![]));
        Named { mon, ot: vec![], nick }
    }

    #[test]
    fn an_evolved_mon_keeps_its_nickname_and_its_lost_hp() {
        let mut pokedex = Pokedex::default();
        let mut named = charmander(Charmander.name());
        named.mon.mon.hp -= 10;
        let before = named.mon.stats[0];
        assert_eq!(evolve(&mut named, Charmeleon, &mut pokedex), None);
        assert_eq!(named.nick, Charmeleon.name(), "the species name follows the species");
        assert_eq!(named.mon.mon.hp, named.mon.stats[0] - 10);
        assert!(named.mon.stats[0] > before);
        assert_eq!(named.mon.mon.types, BaseStats::of(Charmeleon).types);
        assert!(pokedex.is_owned(Charmeleon) && pokedex.is_seen(Charmeleon) && !pokedex.is_owned(Charmander));

        let nick = poke_core::charmap::encode("CHAR").unwrap();
        let mut named = charmander(nick.clone());
        evolve(&mut named, Charmeleon, &mut pokedex);
        assert_eq!(named.nick, nick);
    }
}
