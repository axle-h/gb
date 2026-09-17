//! `LoadEnemyMonData`: the enemy mon about to be sent out.

use poke_core::base_stats::BaseStats;
use poke_core::move_name::PokemonMoveName;
use poke_core::moves::MoveData;
use poke_core::species::PokemonSpecies;
use crate::party::Pokedex;
use crate::rng::Rng;
use crate::systems::add_mon::TRAINER_DVS;
use crate::systems::evos_moves::{write_mon_moves, Learner};
use crate::systems::stats::{calc_stats, Dvs};
use super::{Battle, BattleKind, ExpData, Status3, BASE_STAT_LEVEL};

/// `LoadEnemyMonData` for `wEnemyMonSpecies2` at `wCurEnemyLevel`, from `wWhichPokemon` of a
/// trainer's party. DVs are a transformed mon's own, a trainer's fixed ones, or two random bytes,
/// the second drawn first; stats have no stat experience. A trainer's mon brings its HP, status and
/// moves from its party slot; a wild one starts at full HP with no status, unless transformed, and
/// with its level's moves. PP is each move's base; the species is marked seen.
pub fn load_enemy_mon_data(battle: &mut Battle, pokedex: &mut Pokedex, species: PokemonSpecies, level: u8, which: u8,
                           rng: &mut impl Rng) {
    let base = BaseStats::of(species);
    let transformed = battle.enemy.status3.contains(Status3::TRANSFORMED);
    let trainer = battle.kind == BattleKind::Trainer;
    let dvs = if transformed {
        battle.transformed_enemy_original_dvs
    } else if trainer {
        TRAINER_DVS
    } else {
        let speed_special = rng.random();
        Dvs([rng.random(), speed_special])
    };
    let slot = battle.enemy_party.get(which as usize).cloned();
    let mon = &mut battle.enemy.mon;
    mon.species = species;
    mon.dvs = dvs;
    mon.level = level;
    mon.stats = calc_stats(base.stats, dvs, None, level);
    if trainer {
        let slot = slot.as_ref().expect("a trainer's party slot");
        mon.hp = slot.mon.hp;
        mon.party_pos = which;
        mon.status = slot.mon.status;
    } else if !transformed {
        mon.hp = mon.stats[0];
        mon.status = 0;
    }
    mon.types = base.types;
    mon.catch_rate = base.catch_rate;
    if trainer {
        mon.moves = slot.expect("a trainer's party slot").mon.moves;
    } else {
        mon.moves = base.level_1_moves.map(PokemonMoveName::from_repr);
        let mut unused = [0; 4];
        write_mon_moves(species, level, &mut mon.moves, &mut unused, Learner::New);
    }
    mon.pp = mon.moves.map(|name| name.map_or(0, |name| MoveData::of_move(name).pp));
    battle.enemy_exp = ExpData { base_stats: base.stats, catch_rate: base.catch_rate, base_exp: base.base_exp };
    pokedex.set_seen(species);
    let enemy = &mut battle.enemy;
    enemy.unmodified_level = level;
    enemy.unmodified_stats = enemy.mon.stats;
    enemy.stat_mods = [BASE_STAT_LEVEL; 6];
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use super::super::fixture::each_case;
    use super::*;

    #[test]
    fn every_harvested_case_of_load_enemy_mon_data() {
        each_case(include_str!("../../../fixtures/battle/load_enemy_mon_data.jsonl"), |arena, input, rng| {
            let species = serde_json::from_value(input["species"].clone()).unwrap();
            let byte = |key: &str| input[key].as_u64().unwrap() as u8;
            let mut pokedex = Pokedex::default();
            load_enemy_mon_data(&mut arena.battle, &mut pokedex, species, byte("level"), byte("which"), rng);
            json!(pokedex.seen)
        });
    }
}
