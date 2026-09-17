//! `pokered/fixtures/battle/read_trainer.jsonl`.

use poke_core::species::PokemonSpecies;
use poke_core::trainers::pic_and_money;
use pokered::party::PartyMon;
use pokered::rng::GameRng;
use pokered::systems::add_mon::{new_party_mon, Origin};
use serde_json::{json, Value};
use crate::pokemon::symbols::pokered_symbols as sym;
use super::{changed, read_party, run, Oracle};

const OPP_ID_OFFSET: u8 = 200;
const TRAINER_BATTLE: u8 = 2;

#[derive(Debug, Clone, Copy, serde::Serialize)]
struct Input {
    class: u8,
    trainer_no: u8,
    lone_attack_no: u8,
    rival_starter: u8,
    player_id: u16,
}

/// A party mon as what distinguishes it from the one `new_party_mon` makes of its species and level.
fn against_a_new_mon(mon: &PartyMon, player_id: u16) -> Value {
    let fresh = new_party_mon(mon.mon.species, mon.level, player_id, &Origin::Trainer, &mut GameRng::tape(vec![]));
    let change = changed(&serde_json::to_value(&fresh).unwrap(), &serde_json::to_value(mon).unwrap());
    json!({"species": mon.mon.species, "level": mon.level, "changed": change})
}

/// `ReadTrainer`, with the base money `GetTrainerInformation` would have copied.
fn read_trainer(oracle: &mut Oracle, input: Input) -> (Value, Vec<u8>) {
    let (_, base) = pic_and_money(input.class);
    oracle.write(sym::wLinkState, &[0]);
    oracle.write(sym::wIsInBattle, &[TRAINER_BATTLE]);
    oracle.write(sym::wCurOpponent, &[input.class + OPP_ID_OFFSET]);
    oracle.write(sym::wTrainerClass, &[input.class]);
    oracle.write(sym::wTrainerNo, &[input.trainer_no]);
    oracle.write(sym::wLoneAttackNo, &[input.lone_attack_no]);
    oracle.write(sym::wRivalStarter, &[input.rival_starter]);
    oracle.write(sym::wTrainerBaseMoney, &base[..2]);
    oracle.write(sym::wPlayerID, &input.player_id.to_be_bytes());
    oracle.write(sym::wAmountMoneyWon, &[0xAA; 3]);
    let ran = run(oracle, sym::ReadTrainer, &[], &[], &mut |_, _| {});
    let party = read_party(oracle, sym::wEnemyPartyCount, sym::wEnemyMons);
    let mons: Vec<Value> = party.iter().map(|mon| against_a_new_mon(mon, input.player_id)).collect();
    let money = oracle.read(sym::wAmountMoneyWon, 3);
    (json!({"mons": mons, "money": money}), ran.rng)
}

/// The oracle's own check: Brock's Onix knows Bide in its third slot.
#[test]
fn brock_s_onix_is_given_bide() {
    let mut oracle = super::oracle();
    let (output, _) = read_trainer(&mut oracle, Input { class: 34, trainer_no: 1, lone_attack_no: 1, rival_starter: 0, player_id: 7 });
    assert_eq!(output["mons"][1]["species"], json!(PokemonSpecies::Onix));
    assert_eq!(output["mons"][1]["changed"]["mon"]["moves"]["2"], json!("Bide"));
    assert_eq!(output["money"], json!([0, 0x13, 0x86]), "99 a level for his level 14 Onix");
}

#[cfg(feature = "slow-tests")]
#[test]
#[ignore = "a tool: writes pokered/fixtures/battle/read_trainer.jsonl under GB_REGEN_FIXTURES=1"]
fn harvest_read_trainer() {
    use poke_core::trainers::{party_data, NUM_TRAINERS};
    use rand::RngExt;
    use super::super::{write_fixture, Case};
    use super::generate::{pick, seeded};
    let mut oracle = super::oracle();
    let mut rng = seeded(0x7EA);
    let starters = [PokemonSpecies::Charmander as u8, PokemonSpecies::Squirtle as u8, PokemonSpecies::Bulbasaur as u8];
    let mut cases = vec![];
    for class in 1..=NUM_TRAINERS {
        for (index, (special, _)) in party_data(class).iter().enumerate() {
            let lone_attacks: Vec<u8> = match special {
                true if (34..=40).contains(&class) || class == 29 => (0..=8).collect(),
                true => vec![0, rng.random_range(1..=8)],
                false => vec![0],
            };
            for lone_attack_no in lone_attacks {
                let input = Input {
                    class,
                    trainer_no: index as u8 + 1,
                    lone_attack_no,
                    rival_starter: pick(&mut rng, &starters),
                    player_id: rng.random(),
                };
                let (output, rng_bytes) = read_trainer(&mut oracle, input);
                cases.push(Case { input: serde_json::to_value(input).unwrap(), output, rng: rng_bytes });
            }
        }
    }
    write_fixture("battle", "read_trainer", &cases);
}
