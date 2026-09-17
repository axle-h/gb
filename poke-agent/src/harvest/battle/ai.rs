//! `pokered/fixtures/battle/`: `AIEnemyTrainerChooseMoves`, `SelectEnemyMove` and `TrainerAI`.

use gb::ram::ROM;
use poke_core::item::ItemId;
use pokered::systems::battle::ai::AiAction;
use pokered::systems::battle::{Arena, Side};
use serde_json::{json, Value};
use crate::pokemon::symbols::pokered_symbols as sym;
use super::{run_arena, run_arena_observed, text_printed, Oracle, PRESENTATION};

/// `AIEnemyTrainerChooseMoves`, and the four bytes it leaves `hl` on.
fn ai_enemy_trainer_choose_moves(oracle: &mut Oracle, arena: &Arena) -> (Value, Vec<u8>) {
    let (_, changed, ran) = run_arena(oracle, arena, sym::AIEnemyTrainerChooseMoves, &[], &[]);
    let registers = oracle.registers();
    let hl = u16::from_be_bytes([registers.h, registers.l]);
    let choices = oracle.read(crate::pokemon::symbols::DmgPointer { bank: sym::wBuffer.bank, address: hl }, 4);
    (json!({"changed": changed, "returned": choices}), ran.rng)
}

fn select_enemy_move(oracle: &mut Oracle, arena: &Arena) -> (Value, Vec<u8>) {
    let (_, changed, ran) = run_arena(oracle, arena, sym::SelectEnemyMove, &[], &[]);
    (json!({"changed": changed}), ran.rng)
}

/// `TrainerAI` on the enemy's turn: the item `AIPrintItemUse_` is asked to name, or the switch
/// `EnemySendOut` is asked to carry out, and what was printed.
fn trainer_ai(oracle: &mut Oracle, arena: &Arena) -> (Value, Vec<u8>) {
    oracle.write(sym::hWhoseTurn, &[Side::Enemy as u8]);
    let mut skips = PRESENTATION.to_vec();
    skips.push(("EnemySendOut", sym::EnemySendOut));
    let (mut action, mut texts) = (None, vec![]);
    let (_, changed, ran) = run_arena_observed(oracle, arena, sym::TrainerAI, &[], &skips, &mut |name, gb| {
        text_printed(&mut texts, name, gb);
        match name {
            "AIPrintItemUse_" => {
                let item = gb.core().mmu().read(sym::wAIItem.address);
                action = Some(AiAction::UseItem(ItemId::from_repr(item).expect("an item")));
            }
            "EnemySendOut" => action = Some(AiAction::Switch),
            _ => {}
        }
    });
    assert_eq!(oracle.registers().flags.c, action.is_some(), "carry is an action taken");
    (json!({"changed": changed, "returned": [action, texts]}), ran.rng)
}

/// The oracle's own check: a class with no layers picks from the whole moveset.
#[test]
fn a_youngster_chooses_from_every_move() {
    let mut oracle = super::oracle();
    let mut arena = Arena::baseline();
    arena.battle.trainer_class = 1;
    let (output, _) = ai_enemy_trainer_choose_moves(&mut oracle, &arena);
    let moves: Vec<u8> = arena.battle.enemy.mon.moves.iter().map(|m| m.map_or(0, |m| m as u8)).collect();
    assert_eq!(output["returned"], json!(moves));
}

#[cfg(feature = "slow-tests")]
mod harvest {
    use poke_core::move_name::PokemonMoveName;
    use poke_core::trainers::NUM_TRAINERS;
    use pokered::rng::GameRng;
    use pokered::systems::add_mon::{new_party_mon, Origin};
    use pokered::systems::battle::{status, BattleKind, BattleMon, Combatant, Status1, Status2, Status3};
    use rand::rngs::StdRng;
    use rand::RngExt;
    use super::super::generate::*;
    use super::super::super::{write_fixture, Case};
    use super::super::sparse;
    use super::*;

    fn write(routine: &str, cases: Vec<(Value, (Value, Vec<u8>))>) {
        let cases: Vec<Case<Value, Value>> = cases.into_iter()
            .map(|(input, (output, rng))| Case { input, output, rng })
            .collect();
        write_fixture("battle", routine, &cases);
    }

    /// A moveset with a first move, usually packed from the front, now and then with a gap.
    fn a_moveset(rng: &mut StdRng) -> [Option<PokemonMoveName>; 4] {
        let len = rng.random_range(1..=4);
        let mut moves = [(); 4].map(|_| None);
        for slot in 0..len {
            moves[slot] = Some(a_move(rng));
        }
        if len < 4 && rng.random_bool(0.1) {
            moves[rng.random_range(len..4)] = Some(a_move(rng));
        }
        moves
    }

    fn a_move_choice_arena(rng: &mut StdRng) -> Arena {
        let mut arena = Arena::baseline();
        let battle = &mut arena.battle;
        battle.kind = BattleKind::Trainer;
        battle.trainer_class = rng.random_range(1..=NUM_TRAINERS);
        battle.enemy.mon.moves = a_moveset(rng);
        battle.enemy.disabled_move = if rng.random_bool(0.2) { rng.random_range(1..=4) << 4 | rng.random_range(0..8) } else { 0 };
        battle.player.mon.status = if rng.random_bool(0.4) { a_status(rng) } else { 0 };
        battle.player.mon.types = [a_type(rng), a_type(rng)];
        battle.ai_layer2_encouragement = pick(rng, &[0, 1, 1, 2]);
        arena
    }

    #[test]
    #[ignore = "a tool: writes pokered/fixtures/battle/ai_enemy_trainer_choose_moves.jsonl under GB_REGEN_FIXTURES=1"]
    fn harvest_ai_enemy_trainer_choose_moves() {
        let mut oracle = super::super::oracle();
        let mut rng = seeded(0xA1C);
        let cases = (0..400).map(|_| {
            let arena = a_move_choice_arena(&mut rng);
            (json!({"arena": sparse(&arena)}), ai_enemy_trainer_choose_moves(&mut oracle, &arena))
        }).collect();
        write("ai_enemy_trainer_choose_moves", cases);
    }

    #[test]
    #[ignore = "a tool: writes pokered/fixtures/battle/select_enemy_move.jsonl under GB_REGEN_FIXTURES=1"]
    fn harvest_select_enemy_move() {
        let mut oracle = super::super::oracle();
        let mut rng = seeded(0x5E1);
        let cases = (0..350).map(|_| {
            let mut arena = a_move_choice_arena(&mut rng);
            let battle = &mut arena.battle;
            battle.kind = pick(&mut rng, &[BattleKind::Wild, BattleKind::Trainer]);
            if battle.kind == BattleKind::Wild && battle.enemy.mon.moves[0].is_some() && rng.random_bool(0.3) {
                battle.enemy.mon.moves = [battle.enemy.mon.moves[0], None, None, None];
            }
            let enemy = &mut battle.enemy;
            if rng.random_bool(0.15) {
                enemy.status2 |= pick(&mut rng, &[Status2::NEEDS_TO_RECHARGE, Status2::USING_RAGE]);
            }
            if rng.random_bool(0.15) {
                enemy.status1 |= pick(&mut rng, &[Status1::CHARGING_UP, Status1::THRASHING_ABOUT,
                    Status1::USING_TRAPPING_MOVE, Status1::STORING_ENERGY]);
            }
            enemy.mon.status = if rng.random_bool(0.2) { pick(&mut rng, &[1, 7, status::FRZ, status::PAR]) } else { 0 };
            battle.player.status1.set(Status1::USING_TRAPPING_MOVE, rng.random_bool(0.1));
            (json!({"arena": sparse(&arena)}), select_enemy_move(&mut oracle, &arena))
        }).collect();
        write("select_enemy_move", cases);
    }

    #[test]
    #[ignore = "a tool: writes pokered/fixtures/battle/trainer_ai.jsonl under GB_REGEN_FIXTURES=1"]
    fn harvest_trainer_ai() {
        let mut oracle = super::super::oracle();
        let mut rng = seeded(0x7A1);
        let special: Vec<u8> = (1..=NUM_TRAINERS)
            .filter(|&class| poke_core::trainers::ai_pointer(class).1 != sym::GenericAI.address)
            .collect();
        let (mut cases, mut idle) = (vec![], 0);
        for index in 0.. {
            if cases.len() >= 300 {
                break;
            }
            let mut arena = Arena::baseline();
            arena.badges = rng.random();
            let battle = &mut arena.battle;
            battle.kind = if rng.random_bool(0.05) { BattleKind::Wild } else { BattleKind::Trainer };
            battle.trainer_class = match index % 10 {
                0 => rng.random_range(1..=NUM_TRAINERS),
                1 | 2 => pick(&mut rng, &[42, 43]),
                _ => special[index % special.len()],
            };
            battle.ai_count = pick(&mut rng, &[0, 1, 2, 3, 0xFF, 0xFF, 0xFF, 0xFF]);
            let len = rng.random_range(1..=3);
            battle.enemy_party = (0..len).map(|_| {
                let mut mon = new_party_mon(a_species(&mut rng), rng.random_range(5..=70), 0, &Origin::Trainer, &mut GameRng::tape(vec![]));
                mon.mon.hp = if rng.random_bool(0.4) { 0 } else { rng.random_range(1..=mon.stats[0]) };
                mon.mon.status = if rng.random_bool(0.2) { a_status(&mut rng) } else { 0 };
                mon.mon.box_level = rng.random();
                mon
            }).collect();
            let pos = rng.random_range(0..len);
            let mut out = Combatant::new(BattleMon::from_party(&battle.enemy_party[pos]));
            out.mon.party_pos = pos as u8;
            let max = if rng.random_bool(0.2) { rng.random_range(1..=30) } else { a_safe_stat(&mut rng) };
            out.mon.stats[0] = max;
            out.mon.hp = match rng.random_range(0..6) {
                5 => rng.random_range(0..=max / 4),
                0 => (max / 10).saturating_sub(1) + rng.random_range(0..=2),
                1 => (max / 5).saturating_sub(1) + rng.random_range(0..=2),
                2 => max / 4,
                3 => rng.random_range(0..=max),
                _ => max.saturating_sub(rng.random_range(0..=30)),
            };
            out.mon.status = if rng.random_bool(0.4) { a_status(&mut rng) } else { 0 };
            out.status3.set(Status3::BADLY_POISONED, rng.random_bool(0.3));
            out.stat_mods = [(); 6].map(|_| a_stat_mod(&mut rng));
            out.mon.stats[1..].iter_mut().for_each(|stat| *stat = if rng.random_bool(0.2) { 999 } else { a_safe_stat(&mut rng) });
            out.unmodified_stats = [(); 5].map(|_| a_safe_stat(&mut rng));
            out.current_move = poke_core::moves::MoveData::of_move(a_move(&mut rng));
            battle.enemy = out;
            battle.player.mon.status = if rng.random_bool(0.3) { status::PAR | status::BRN } else { 0 };
            let (output, rng_bytes) = trainer_ai(&mut oracle, &arena);
            // Most turns an AI does nothing: keep enough of those, and every one where it acts.
            if output["returned"][0].is_null() {
                idle += 1;
                if idle > 80 {
                    continue;
                }
            }
            cases.push((json!({"arena": sparse(&arena)}), (output, rng_bytes)));
        }
        write("trainer_ai", cases);
    }
}
