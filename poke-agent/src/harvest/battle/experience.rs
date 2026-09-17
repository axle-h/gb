//! `pokered/fixtures/battle/`: `GainExperience`, and `FaintEnemyPokemon`'s Exp. All.

use gb::game_boy::GameBoy;
use gb::ram::ROM;
use poke_core::item::ItemId;
use pokered::systems::battle::experience::ExpEvent;
use pokered::systems::battle::Arena;
use serde_json::{json, Value};
use crate::pokemon::symbols::{pokered_symbols as sym, DmgPointer};
use super::{local_label, run_arena_observed, Oracle};

/// Everything `GainExperience` calls that prints, draws or waits.
const SKIPS: [(&str, DmgPointer); 8] = [
    ("PrintText", sym::PrintText),
    ("DrawPlayerHUDAndHPBar", sym::DrawPlayerHUDAndHPBar),
    ("PrintEmptyString", sym::PrintEmptyString),
    ("SaveScreenTilesToBuffer1", sym::SaveScreenTilesToBuffer1),
    ("PrintStatsBox", sym::PrintStatsBox),
    ("WaitForTextScrollButtonPress", sym::WaitForTextScrollButtonPress),
    ("LoadScreenTilesFromBuffer1", sym::LoadScreenTilesFromBuffer1),
    ("LearnMoveFromLevelUp", sym::LearnMoveFromLevelUp),
];

/// The two texts `GainExperience` prints, read off the machine as `PrintText` is called for them.
fn observe(events: &mut Vec<ExpEvent>, name: &str, gb: &GameBoy) {
    let mmu = gb.core().mmu();
    let hl = u16::from_be_bytes([gb.core().registers().h, gb.core().registers().l]);
    let slot = mmu.read(sym::wWhichPokemon.address);
    if name != "PrintText" {
        return;
    }
    if hl == sym::GainedText.address {
        let amount = u16::from_be_bytes([mmu.read(sym::wExpAmountGained.address), mmu.read(sym::wExpAmountGained.address + 1)]);
        events.push(ExpEvent::Gained { slot, amount, boosted: mmu.read(sym::wGainBoostedExp.address) != 0 });
    } else if hl == sym::GrewLevelText.address {
        events.push(ExpEvent::GrewLevel { slot, level: mmu.read(sym::wCurEnemyLevel.address) });
    }
}

fn gain_experience(oracle: &mut Oracle, arena: &Arena) -> (Value, Vec<u8>) {
    let mut events = vec![];
    let (_, changed, ran) = run_arena_observed(oracle, arena, sym::GainExperience, &[], &SKIPS,
        &mut |name, gb| observe(&mut events, name, gb));
    (json!({"changed": changed, "returned": events}), ran.rng)
}

/// `FaintEnemyPokemon` from `.playermonnotfaint`, with a bag that holds an Exp. All or nothing.
fn faint_enemy_pokemon_experience(oracle: &mut Oracle, arena: &Arena, has_exp_all: bool) -> (Value, Vec<u8>) {
    let bag: &[u8] = if has_exp_all { &[ItemId::ExpAll as u8, 1, 0xFF] } else { &[0xFF] };
    oracle.write(sym::wNumBagItems, &[has_exp_all as u8]);
    oracle.write(sym::wBagItems, bag);
    let mut events = vec![];
    let (_, changed, ran) = run_arena_observed(oracle, arena, local_label("FaintEnemyPokemon.playermonnotfaint"), &[],
        &SKIPS, &mut |name, gb| observe(&mut events, name, gb));
    (json!({"changed": changed, "returned": events}), ran.rng)
}

/// The oracle's own check: a lone wild Rattata at level 50 is worth `57 * 50 / 7` to the Tauros.
#[test]
fn a_wild_rattata_is_worth_its_base_experience() {
    let mut oracle = super::oracle();
    let mut arena = Arena::baseline();
    arena.battle.enemy_exp.base_exp = 57;
    arena.battle.gain_exp_flags = 1;
    let (output, _) = gain_experience(&mut oracle, &arena);
    let exp = arena.party[0].mon.exp as u64 + 57 * 50 / 7;
    assert_eq!(output["changed"]["party"]["0"]["mon"]["exp"], json!(exp));
    assert_eq!(output["returned"][0], json!({"Gained": {"slot": 0, "amount": 57 * 50 / 7, "boosted": false}}));
}

#[cfg(feature = "slow-tests")]
mod harvest {
    use poke_core::base_stats::BaseStats;
    use pokered::party::PartyMon;
    use pokered::rng::GameRng;
    use pokered::systems::add_mon::{new_party_mon, Origin};
    use pokered::systems::battle::{BattleKind, BattleMon, Combatant, Status3};
    use pokered::systems::experience::calc_experience;
    use pokered::systems::stats::Dvs;
    use rand::rngs::StdRng;
    use rand::RngExt;
    use super::super::generate::*;
    use super::super::super::{write_fixture, Case};
    use super::*;

    const PLAYER_ID: u16 = 0x1234;

    /// A mon a battle could leave behind: often one hit short of its next level, sometimes fainted,
    /// traded or at the top of the experience curve.
    fn a_party_mon(rng: &mut StdRng) -> PartyMon {
        let species = a_species(rng);
        let growth = BaseStats::of(species).growth_rate;
        let level = if rng.random_bool(0.1) { 100 } else { rng.random_range(2..=99) };
        let ot_id = if rng.random_bool(0.25) { rng.random() } else { PLAYER_ID };
        let mut mon = new_party_mon(species, level, ot_id, &Origin::Trainer, &mut GameRng::tape(vec![]));
        let (this, next) = (calc_experience(growth, level), calc_experience(growth, (level + 1).min(100)));
        mon.mon.exp = match rng.random_range(0..3) {
            0 => next.saturating_sub(rng.random_range(1..=200)).max(this),
            1 => this + (next.saturating_sub(this)) / 2,
            _ => this,
        };
        mon.mon.dvs = Dvs([rng.random(), rng.random()]);
        mon.mon.stat_exp = [(); 5].map(|_| if rng.random_bool(0.2) { rng.random_range(0xFF00..=0xFFFF) } else { rng.random() });
        mon.mon.hp = match rng.random_range(0..6) {
            0 => 0,
            1 => 1,
            _ => rng.random_range(1..=mon.stats[0]),
        };
        mon
    }

    fn an_experience_arena(rng: &mut StdRng) -> Arena {
        let mut arena = Arena::baseline();
        arena.player_id = PLAYER_ID;
        arena.badges = rng.random();
        let len = rng.random_range(1..=6);
        arena.party = (0..len).map(|_| a_party_mon(rng)).collect();
        let battle = &mut arena.battle;
        battle.kind = pick(rng, &[BattleKind::Wild, BattleKind::Trainer]);
        battle.player_mon_number = rng.random_range(0..len as u8);
        let out = &arena.party[battle.player_mon_number as usize];
        battle.player = Combatant::new(BattleMon::from_party(out));
        battle.player.mon.status = a_status(rng);
        battle.player.stat_mods[..4].iter_mut().for_each(|stat_mod| *stat_mod = a_stat_mod(rng));
        if rng.random_bool(0.2) {
            battle.player.status3 |= Status3::TRANSFORMED;
            battle.player.unmodified_stats = [(); 5].map(|_| a_safe_stat(rng));
        }
        battle.enemy.mon.level = a_level(rng);
        battle.enemy_exp.base_stats = [(); 5].map(|_| rng.random());
        battle.enemy_exp.catch_rate = rng.random();
        battle.enemy_exp.base_exp = if rng.random_bool(0.2) { 255 } else { rng.random() };
        battle.gain_exp_flags = match rng.random_range(0..4) {
            0 => 1 << battle.player_mon_number,
            1 => rng.random(),
            _ => rng.random::<u8>() & ((1u16 << len) - 1) as u8,
        };
        battle.fought_current_enemy_flags = rng.random();
        battle.can_evolve_flags = rng.random();
        arena
    }

    fn write(routine: &str, cases: Vec<(Value, (Value, Vec<u8>))>) {
        let cases: Vec<Case<Value, Value>> = cases.into_iter()
            .map(|(input, (output, rng))| Case { input, output, rng })
            .collect();
        write_fixture("battle", routine, &cases);
    }

    #[test]
    #[ignore = "a tool: writes pokered/fixtures/battle/gain_experience.jsonl under GB_REGEN_FIXTURES=1"]
    fn harvest_gain_experience() {
        let mut oracle = super::super::oracle();
        let mut rng = seeded(0xE4);
        let cases = (0..200).map(|_| {
            let arena = an_experience_arena(&mut rng);
            (json!({"arena": super::super::sparse(&arena)}), gain_experience(&mut oracle, &arena))
        }).collect();
        write("gain_experience", cases);
    }

    #[test]
    #[ignore = "a tool: writes pokered/fixtures/battle/faint_enemy_pokemon_experience.jsonl under GB_REGEN_FIXTURES=1"]
    fn harvest_faint_enemy_pokemon_experience() {
        let mut oracle = super::super::oracle();
        let mut rng = seeded(0xE4A11);
        let cases = (0..120).map(|_| {
            let mut arena = an_experience_arena(&mut rng);
            if rng.random_bool(0.05) {
                arena.party.iter_mut().for_each(|mon| mon.mon.hp = 0);
            }
            let has_exp_all = rng.random_bool(0.7);
            let input = json!({"arena": super::super::sparse(&arena), "has_exp_all": has_exp_all});
            (input, faint_enemy_pokemon_experience(&mut oracle, &arena, has_exp_all))
        }).collect();
        write("faint_enemy_pokemon_experience", cases);
    }
}
