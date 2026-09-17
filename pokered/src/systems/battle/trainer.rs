//! `ReadTrainer`: a trainer's party from `TrainerDataPointers`, the gym leaders' and the Elite
//! Four's special moves, and the prize money.

use poke_core::move_name::PokemonMoveName;
use poke_core::species::PokemonSpecies;
use poke_core::trainers::{lone_moves, party_data, pic_and_money, team_moves};
use serde::{Deserialize, Serialize};
use crate::party::PartyMon;
use crate::rng::GameRng;
use crate::systems::add_mon::{new_party_mon, Origin};
use crate::systems::math::add_bcd;

/// `RIVAL3`, the champion.
pub const RIVAL3: u8 = 43;
/// `STARTER1` and `STARTER3`: the rival's starter decides his last mon's move.
const STARTER1: PokemonSpecies = PokemonSpecies::Charmander;
const STARTER3: PokemonSpecies = PokemonSpecies::Bulbasaur;
/// The third move slot, where every special move goes.
const SPECIAL_MOVE_SLOT: usize = 2;

/// What `ReadTrainer` leaves: `wEnemyMons` and `wAmountMoneyWon`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrainerParty {
    pub mons: Vec<PartyMon>,
    /// BCD, three bytes.
    pub money: [u8; 3],
}

/// `ReadTrainer` for `wTrainerClass` from 1 and `wTrainerNo` from 1. A party whose mons each have
/// their own level can carry a special move into the third slot, replacing the move but not its
/// PP: `wLoneAttackNo`'s entry for a gym leader, and otherwise the Elite Four's fifth mon or the
/// champion's first and sixth. The prize is the class's base money once per level of the *last*
/// mon, in two BCD bytes that stop at 9999.
pub fn read_trainer(class: u8, trainer_no: u8, lone_attack_no: u8, rival_starter: u8, player_id: u16) -> TrainerParty {
    let (per_mon_levels, data) = &party_data(class)[trainer_no as usize - 1];
    let mut mons: Vec<PartyMon> = data.iter()
        .map(|&(level, species)| {
            let species = PokemonSpecies::from_repr(species).expect("a trainer's species");
            new_party_mon(species, level, player_id, &Origin::Trainer, &mut GameRng::tape(vec![]))
        })
        .collect();
    if *per_mon_levels {
        // Into a party too short for it the cartridge writes past the last mon, where nothing reads.
        let mut teach = |index: usize, name: u8| {
            if let Some(mon) = mons.get_mut(index) {
                mon.mon.moves[SPECIAL_MOVE_SLOT] = PokemonMoveName::from_repr(name);
            }
        };
        if lone_attack_no != 0 {
            let (index, name) = lone_moves()[lone_attack_no as usize - 1];
            teach(index as usize, name);
        } else if let Some(&(_, name)) = team_moves().iter().find(|&&(team, _)| team == class) {
            teach(4, name);
        } else if class == RIVAL3 {
            teach(0, PokemonMoveName::SkyAttack as u8);
            let starter_move = match PokemonSpecies::from_repr(rival_starter) {
                Some(STARTER3) => PokemonMoveName::MegaDrain,
                Some(STARTER1) => PokemonMoveName::FireBlast,
                _ => PokemonMoveName::Blizzard,
            };
            teach(5, starter_move as u8);
        }
    }
    let (_, base) = pic_and_money(class);
    let mut money = [0; 3];
    let last_level = data.last().expect("a party").0;
    for _ in 0..if last_level == 0 { 256 } else { last_level as u32 } {
        add_bcd(&mut money[1..], &base[..2]);
    }
    TrainerParty { mons, money }
}

#[cfg(test)]
mod tests {
    use serde_json::Value;
    use crate::fixtures::cases;
    use super::super::fixture::assert_changed;
    use super::*;

    #[test]
    fn every_harvested_case_of_read_trainer() {
        for (input, output, _) in cases::<Value, Value>(include_str!("../../../fixtures/battle/read_trainer.jsonl")) {
            let byte = |key: &str| input[key].as_u64().unwrap() as u8;
            let player_id = input["player_id"].as_u64().unwrap() as u16;
            let party = read_trainer(byte("class"), byte("trainer_no"), byte("lone_attack_no"), byte("rival_starter"), player_id);
            let expected = output["mons"].as_array().unwrap();
            assert_eq!(party.mons.len(), expected.len(), "{input}");
            for (mon, expected) in party.mons.iter().zip(expected) {
                let species = serde_json::from_value(expected["species"].clone()).unwrap();
                let level = expected["level"].as_u64().unwrap() as u8;
                let fresh = new_party_mon(species, level, player_id, &Origin::Trainer, &mut GameRng::tape(vec![]));
                assert_changed(&fresh, mon, &expected["changed"], &input.to_string());
            }
            assert_eq!(serde_json::to_value(party.money).unwrap(), output["money"], "{input}");
        }
    }
}
