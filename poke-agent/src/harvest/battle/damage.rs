//! `pokered/fixtures/battle/`: the damage, accuracy, stat and turn-order routines.

use pokered::systems::battle::damage::DamageVars;
use pokered::systems::battle::{Arena, Side};
use serde_json::{json, Value};
use crate::pokemon::symbols::{pokered_symbols as sym, DmgPointer};
use super::{local_label, run_arena, sparse, Oracle};

fn whose_turn(oracle: &mut Oracle, side: Side) {
    oracle.write(sym::hWhoseTurn, &[side as u8]);
}

/// A routine that takes only `hWhoseTurn`: what it changed, and its random bytes.
fn on_turn(oracle: &mut Oracle, arena: &Arena, side: Side, routine: DmgPointer) -> (Value, Vec<u8>) {
    whose_turn(oracle, side);
    let (_, changed, ran) = run_arena(oracle, arena, routine, &[], &[]);
    (json!({"changed": changed}), ran.rng)
}

fn critical_hit_test(oracle: &mut Oracle, arena: &Arena, side: Side) -> (Value, Vec<u8>) {
    on_turn(oracle, arena, side, sym::CriticalHitTest)
}

/// `GetDamageVarsFor*Attack`: `b`, `c`, `d` and `e`, or `None` on its `ret z`.
fn get_damage_vars(oracle: &mut Oracle, arena: &Arena, side: Side) -> (Value, Vec<u8>) {
    let routine = match side {
        Side::Player => sym::GetDamageVarsForPlayerAttack,
        Side::Enemy => sym::GetDamageVarsForEnemyAttack,
    };
    let (_, changed, ran) = run_arena(oracle, arena, routine, &[], &[]);
    let r = oracle.registers();
    let vars = (!r.flags.z).then_some(DamageVars { attack: r.b, defense: r.c, power: r.d, level: r.e });
    (json!({"changed": changed, "returned": vars}), ran.rng)
}

/// `CalculateDamage` with its register arguments, and whether it left `z`.
fn calculate_damage(oracle: &mut Oracle, arena: &Arena, side: Side, vars: DamageVars) -> (Value, Vec<u8>) {
    whose_turn(oracle, side);
    let registers = oracle.registers_mut();
    (registers.b, registers.c, registers.d, registers.e) = (vars.attack, vars.defense, vars.power, vars.level);
    let (_, changed, ran) = run_arena(oracle, arena, sym::CalculateDamage, &[], &[]);
    let skipped = oracle.registers().flags.z;
    (json!({"changed": changed, "returned": if skipped { "Skipped" } else { "Damage" }}), ran.rng)
}

fn handle_counter_move(oracle: &mut Oracle, arena: &Arena, side: Side) -> (Value, Vec<u8>) {
    whose_turn(oracle, side);
    let (_, changed, ran) = run_arena(oracle, arena, sym::HandleCounterMove, &[], &[]);
    let resolved = oracle.registers().flags.z;
    (json!({"changed": changed, "returned": if resolved { "Resolved" } else { "NotCounter" }}), ran.rng)
}

fn ai_get_type_effectiveness(oracle: &mut Oracle, arena: &Arena) -> (Value, Vec<u8>) {
    let (_, changed, ran) = run_arena(oracle, arena, sym::AIGetTypeEffectiveness, &[], &[]);
    let effectiveness = oracle.read(sym::wTypeEffectiveness, 1)[0];
    (json!({"changed": changed, "returned": effectiveness}), ran.rng)
}

/// `CalculateModifiedStat` for `wCalculateWhoseStats` and `c`.
fn calculate_modified_stat(oracle: &mut Oracle, arena: &Arena, whose: Side, which: u8) -> (Value, Vec<u8>) {
    oracle.write(sym::wCalculateWhoseStats, &[whose as u8]);
    oracle.registers_mut().c = which;
    let (_, changed, ran) = run_arena(oracle, arena, sym::CalculateModifiedStat, &[], &[]);
    (json!({"changed": changed}), ran.rng)
}

fn apply_burn_and_paralysis_penalties(oracle: &mut Oracle, arena: &Arena, to: Side) -> (Value, Vec<u8>) {
    let routine = match to {
        Side::Player => sym::ApplyBurnAndParalysisPenaltiesToPlayer,
        Side::Enemy => sym::ApplyBurnAndParalysisPenaltiesToEnemy,
    };
    let (_, changed, ran) = run_arena(oracle, arena, routine, &[], &[]);
    (json!({"changed": changed}), ran.rng)
}

fn apply_badge_stat_boosts(oracle: &mut Oracle, arena: &Arena) -> (Value, Vec<u8>) {
    let (_, changed, ran) = run_arena(oracle, arena, sym::ApplyBadgeStatBoosts, &[], &[]);
    (json!({"changed": changed}), ran.rng)
}

/// `MainInBattleLoop` from `.noLinkBattle` to whichever side it sends first.
fn first_to_move(oracle: &mut Oracle, arena: &Arena) -> (Value, Vec<u8>) {
    let stops = [
        ("Player", local_label("MainInBattleLoop.playerMovesFirst")),
        ("Enemy", local_label("MainInBattleLoop.enemyMovesFirst")),
    ];
    let (_, changed, ran) = run_arena(oracle, arena, local_label("MainInBattleLoop.noLinkBattle"), &stops, &[]);
    (json!({"changed": changed, "returned": ran.exit.expect("a side moves first")}), ran.rng)
}

/// `PlayerCalcMoveDamage` or `EnemyCalcMoveDamage` to the first label after it where the levels
/// the enemy's copy swaps are back in place.
fn calc_move_damage(oracle: &mut Oracle, arena: &Arena, side: Side) -> (Value, Vec<u8>) {
    whose_turn(oracle, side);
    let (entry, stops) = match side {
        Side::Player => (sym::PlayerCalcMoveDamage, [
            ("Hit", sym::GetPlayerAnimationType),
            ("ExplosionMissed", sym::PlayPlayerMoveAnimation),
            ("NotAnimated", sym::PlayerCheckIfFlyOrChargeEffect),
        ]),
        Side::Enemy => (sym::EnemyCalcMoveDamage, [
            ("Hit", sym::GetEnemyAnimationType),
            ("ExplosionMissed", sym::PlayEnemyMoveAnimation),
            ("NotAnimated", sym::DelayFrames),
        ]),
    };
    let (_, changed, ran) = run_arena(oracle, arena, entry, &stops, &[]);
    (json!({"changed": changed, "returned": ran.exit.expect("an exit")}), ran.rng)
}

fn input(side: Option<Side>, arena: &Arena) -> Value {
    json!({"side": side, "arena": sparse(arena)})
}

/// The oracle's own check: a few values worked out by hand.
#[test]
fn the_damage_routines_answer_as_the_formulas_do() {
    let mut oracle = super::oracle();
    let arena = Arena::baseline();
    let vars = DamageVars { attack: 100, defense: 100, power: 40, level: 50 };
    let (output, _) = calculate_damage(&mut oracle, &arena, Side::Player, vars);
    assert_eq!(output["changed"]["battle"]["damage"], json!((22 * 40 * 100 / 100) / 50 + 2));
    let mut paralysed = arena.clone();
    paralysed.battle.enemy.mon.status = pokered::systems::battle::status::PAR;
    let (output, _) = apply_burn_and_paralysis_penalties(&mut oracle, &paralysed, Side::Enemy);
    let speed = arena.battle.enemy.mon.stats[3];
    assert_eq!(output["changed"]["battle"]["enemy"]["mon"]["stats"]["3"], json!(speed / 4));
}

#[cfg(feature = "slow-tests")]
mod harvest {
    use poke_core::battle_data::high_critical_moves;
    use poke_core::move_name::PokemonMoveName;
    use poke_core::moves::MoveData;
    use pokered::systems::battle::{effect, CriticalHitOrOhko, Status1, Status2, Status3};
    use rand::rngs::StdRng;
    use rand::RngExt;
    use super::super::generate::*;
    use super::super::super::{write_fixture, Case};
    use super::*;

    fn write(routine: &str, cases: Vec<(Value, (Value, Vec<u8>))>) {
        let cases: Vec<Case<Value, Value>> = cases.into_iter()
            .map(|(input, (output, rng))| Case { input, output, rng })
            .collect();
        write_fixture("battle", routine, &cases);
    }

    fn a_move_of_power(rng: &mut StdRng) -> PokemonMoveName {
        loop {
            let name = a_move(rng);
            if MoveData::of_move(name).power > 1 {
                return name;
            }
        }
    }

    #[test]
    #[ignore = "a tool: writes pokered/fixtures/battle/critical_hit_test.jsonl under GB_REGEN_FIXTURES=1"]
    fn harvest_critical_hit_test() {
        let mut oracle = super::super::oracle();
        let mut rng = seeded(0xC417);
        let high: Vec<u8> = high_critical_moves();
        let cases = (0..400).map(|i| {
            let side = a_side(&mut rng);
            let mut arena = Arena::baseline();
            arena.battle.side_mut(side).mon.species = a_species(&mut rng);
            let name = match i % 4 {
                0 => PokemonMoveName::from_repr(pick(&mut rng, &high)).unwrap(),
                _ => a_move(&mut rng),
            };
            using(&mut arena, side, name);
            if rng.random_bool(0.3) {
                arena.battle.side_mut(side).status2 |= Status2::GETTING_PUMPED;
            }
            if rng.random_bool(0.2) {
                arena.battle.critical_hit_or_ohko = CriticalHitOrOhko::CriticalHit;
            }
            (input(Some(side), &arena), critical_hit_test(&mut oracle, &arena, side))
        }).collect();
        write("critical_hit_test", cases);
    }

    #[test]
    #[ignore = "a tool: writes pokered/fixtures/battle/get_damage_vars.jsonl under GB_REGEN_FIXTURES=1"]
    fn harvest_get_damage_vars() {
        let mut oracle = super::super::oracle();
        let mut rng = seeded(0xDA7A);
        let cases = (0..350).map(|i| {
            let side = a_side(&mut rng);
            let mut arena = Arena::baseline();
            using(&mut arena, side, if i % 10 == 0 { a_move(&mut rng) } else { a_move_of_power(&mut rng) });
            let battle = &mut arena.battle;
            battle.damage = a_damage(&mut rng);
            for combatant in [&mut battle.player, &mut battle.enemy] {
                combatant.mon.stats = [(); 5].map(|_| a_stat(&mut rng));
                combatant.mon.level = a_level(&mut rng);
                if rng.random_bool(0.4) {
                    combatant.status3 = Status3::from_bits_retain(rng.random_range(0..16));
                }
            }
            battle.enemy.mon.species = a_species(&mut rng);
            battle.enemy.mon.dvs = pokered::systems::stats::Dvs([rng.random(), rng.random()]);
            battle.critical_hit_or_ohko = pick(&mut rng, &[CriticalHitOrOhko::Normal, CriticalHitOrOhko::Normal,
                CriticalHitOrOhko::CriticalHit, CriticalHitOrOhko::CriticalHit, CriticalHitOrOhko::SuccessfulOhko,
                CriticalHitOrOhko::FailedOhko]);
            arena.party[0].stats = [(); 5].map(|_| a_stat(&mut rng));
            (input(Some(side), &arena), get_damage_vars(&mut oracle, &arena, side))
        }).collect();
        write("get_damage_vars", cases);
    }

    #[test]
    #[ignore = "a tool: writes pokered/fixtures/battle/calculate_damage.jsonl under GB_REGEN_FIXTURES=1"]
    fn harvest_calculate_damage() {
        let mut oracle = super::super::oracle();
        let mut rng = seeded(0xCA1C);
        let effects = [effect::NO_ADDITIONAL_EFFECT, effect::EXPLODE_EFFECT, effect::TWO_TO_FIVE_ATTACKS_EFFECT,
            effect::EFFECT_1E, effect::OHKO_EFFECT, effect::RECOIL_EFFECT, effect::DRAIN_HP_EFFECT];
        let cases = (0..400).map(|i| {
            let side = a_side(&mut rng);
            let mut arena = Arena::baseline();
            let move_effect = if i < 40 { effect::NO_ADDITIONAL_EFFECT } else { pick(&mut rng, &effects) };
            arena.battle.side_mut(side).current_move.effect = move_effect;
            arena.battle.damage = if rng.random_bool(0.7) { 0 } else { a_damage(&mut rng) };
            arena.battle.player.mon.stats[3] = a_stat(&mut rng);
            arena.battle.enemy.mon.stats[3] = if rng.random_bool(0.2) { arena.battle.player.mon.stats[3] } else { a_stat(&mut rng) };
            let byte = |rng: &mut StdRng| if rng.random_bool(0.2) { pick(rng, &[1, 2, 127, 128, 254, 255]) } else { rng.random_range(1..=255) };
            let mut vars = DamageVars { attack: byte(&mut rng), defense: byte(&mut rng), power: byte(&mut rng), level: byte(&mut rng) };
            if i < 40 {
                vars = DamageVars { attack: 255, defense: [1, 2, 3, 255][i % 4], power: 255, level: 255 };
            }
            if rng.random_bool(0.05) {
                vars.power = 0;
            }
            if move_effect == effect::EXPLODE_EFFECT && rng.random_bool(0.3) {
                vars.defense = rng.random_range(0..=1);
            }
            let input = json!({"side": side, "arena": sparse(&arena), "vars": vars});
            (input, calculate_damage(&mut oracle, &arena, side, vars))
        }).collect();
        write("calculate_damage", cases);
    }

    #[test]
    #[ignore = "a tool: writes pokered/fixtures/battle/adjust_damage_for_move_type.jsonl under GB_REGEN_FIXTURES=1"]
    fn harvest_adjust_damage_for_move_type() {
        let mut oracle = super::super::oracle();
        let mut rng = seeded(0x7E5);
        let cases = (0..400).map(|_| {
            let side = a_side(&mut rng);
            let mut arena = Arena::baseline();
            let battle = &mut arena.battle;
            battle.player.mon.types = [a_type(&mut rng), a_type(&mut rng)];
            battle.enemy.mon.types = if rng.random_bool(0.3) { [a_type(&mut rng); 2] } else { [a_type(&mut rng), a_type(&mut rng)] };
            battle.side_mut(side).current_move.move_type = a_type(&mut rng);
            battle.damage = a_damage(&mut rng);
            battle.damage_multipliers = if rng.random_bool(0.5) { 10 } else { rng.random() };
            battle.move_missed = rng.random_bool(0.1);
            (input(Some(side), &arena), on_turn(&mut oracle, &arena, side, sym::AdjustDamageForMoveType))
        }).collect();
        write("adjust_damage_for_move_type", cases);
    }

    #[test]
    #[ignore = "a tool: writes pokered/fixtures/battle/randomize_damage.jsonl under GB_REGEN_FIXTURES=1"]
    fn harvest_randomize_damage() {
        let mut oracle = super::super::oracle();
        let mut rng = seeded(0x2A2D);
        let cases = (0..300).map(|_| {
            let mut arena = Arena::baseline();
            arena.battle.damage = a_damage(&mut rng);
            (input(None, &arena), on_turn(&mut oracle, &arena, Side::Player, sym::RandomizeDamage))
        }).collect();
        write("randomize_damage", cases);
    }

    /// The fields `MoveHitTest` reads, at random.
    fn hit_test_arena(rng: &mut StdRng, side: Side, arena: &mut Arena) {
        let effects = [effect::DREAM_EATER_EFFECT, effect::SWIFT_EFFECT, effect::DRAIN_HP_EFFECT,
            effect::ATTACK_DOWN1_EFFECT, effect::HAZE_EFFECT, effect::ATTACK_DOWN2_EFFECT, effect::REFLECT_EFFECT,
            effect::EVASION_UP1_EFFECT, effect::BIDE_EFFECT, effect::LIGHT_SCREEN_EFFECT, effect::POISON_EFFECT];
        let battle = &mut arena.battle;
        let move_effect = if rng.random_bool(0.5) { pick(rng, &effects) } else { rng.random_range(0..=0x56) };
        battle.side_mut(side).current_move.effect = move_effect;
        battle.side_mut(side).current_move.accuracy = if rng.random_bool(0.3) { pick(rng, &[0, 1, 127, 254, 255]) } else { rng.random() };
        battle.damage = a_damage(rng);
        for combatant in [&mut battle.player, &mut battle.enemy] {
            combatant.stat_mods[4] = a_stat_mod(rng);
            combatant.stat_mods[5] = a_stat_mod(rng);
            if rng.random_bool(0.3) {
                combatant.mon.status = a_status(rng);
            }
            for (flag, chance) in [(Status1::INVULNERABLE, 0.1), (Status1::USING_TRAPPING_MOVE, 0.3)] {
                combatant.status1.set(flag, rng.random_bool(chance));
            }
            for (flag, chance) in [(Status2::PROTECTED_BY_MIST, 0.2), (Status2::USING_X_ACCURACY, 0.15),
                                   (Status2::HAS_SUBSTITUTE_UP, 0.2)] {
                combatant.status2.set(flag, rng.random_bool(chance));
            }
        }
    }

    #[test]
    #[ignore = "a tool: writes pokered/fixtures/battle/move_hit_test.jsonl under GB_REGEN_FIXTURES=1"]
    fn harvest_move_hit_test() {
        let mut oracle = super::super::oracle();
        let mut rng = seeded(0x417);
        let cases = (0..350).map(|_| {
            let side = a_side(&mut rng);
            let mut arena = Arena::baseline();
            hit_test_arena(&mut rng, side, &mut arena);
            (input(Some(side), &arena), on_turn(&mut oracle, &arena, side, sym::MoveHitTest))
        }).collect();
        write("move_hit_test", cases);
    }

    #[test]
    #[ignore = "a tool: writes pokered/fixtures/battle/calc_hit_chance.jsonl under GB_REGEN_FIXTURES=1"]
    fn harvest_calc_hit_chance() {
        let mut oracle = super::super::oracle();
        let mut rng = seeded(0xACC);
        let mut cases = vec![];
        for accuracy_mod in 1..=13 {
            for evasion_mod in 1..=13 {
                let side = a_side(&mut rng);
                let mut arena = Arena::baseline();
                arena.battle.side_mut(side).stat_mods[4] = accuracy_mod;
                arena.battle.side_mut(side.other()).stat_mods[5] = evasion_mod;
                arena.battle.side_mut(side).current_move.accuracy = pick(&mut rng, &[1, 2, 3, 76, 127, 178, 229, 242, 255]);
                cases.push((input(Some(side), &arena), on_turn(&mut oracle, &arena, side, sym::CalcHitChance)));
            }
        }
        for _ in 0..231 {
            let side = a_side(&mut rng);
            let mut arena = Arena::baseline();
            arena.battle.side_mut(side).stat_mods[4] = a_stat_mod(&mut rng);
            arena.battle.side_mut(side.other()).stat_mods[5] = a_stat_mod(&mut rng);
            arena.battle.side_mut(side).current_move.accuracy = rng.random();
            cases.push((input(Some(side), &arena), on_turn(&mut oracle, &arena, side, sym::CalcHitChance)));
        }
        write("calc_hit_chance", cases);
    }

    #[test]
    #[ignore = "a tool: writes pokered/fixtures/battle/handle_counter_move.jsonl under GB_REGEN_FIXTURES=1"]
    fn harvest_handle_counter_move() {
        let mut oracle = super::super::oracle();
        let mut rng = seeded(0xC0);
        let cases = (0..400).map(|_| {
            let side = a_side(&mut rng);
            let mut arena = Arena::baseline();
            hit_test_arena(&mut rng, side, &mut arena);
            let counter = PokemonMoveName::Counter as u8;
            let battle = &mut arena.battle;
            battle.side_mut(side).selected_move = if rng.random_bool(0.8) { counter } else { a_move(&mut rng) as u8 };
            let target = battle.side_mut(side.other());
            target.selected_move = if rng.random_bool(0.15) { counter } else { a_move(&mut rng) as u8 };
            target.current_move.power = if rng.random_bool(0.2) { 0 } else { rng.random() };
            target.current_move.move_type = if rng.random_bool(0.6) { rng.random_range(0..=1) } else { a_type(&mut rng) };
            (input(Some(side), &arena), handle_counter_move(&mut oracle, &arena, side))
        }).collect();
        write("handle_counter_move", cases);
    }

    #[test]
    #[ignore = "a tool: writes pokered/fixtures/battle/ai_get_type_effectiveness.jsonl under GB_REGEN_FIXTURES=1"]
    fn harvest_ai_get_type_effectiveness() {
        let mut oracle = super::super::oracle();
        let mut rng = seeded(0xA17);
        let cases = (0..400).map(|_| {
            let mut arena = Arena::baseline();
            arena.battle.enemy.current_move.move_type = a_type(&mut rng);
            arena.battle.player.mon.types = [a_type(&mut rng), a_type(&mut rng)];
            (input(None, &arena), ai_get_type_effectiveness(&mut oracle, &arena))
        }).collect();
        write("ai_get_type_effectiveness", cases);
    }

    #[test]
    #[ignore = "a tool: writes pokered/fixtures/battle/calculate_modified_stat.jsonl under GB_REGEN_FIXTURES=1"]
    fn harvest_calculate_modified_stat() {
        let mut oracle = super::super::oracle();
        let mut rng = seeded(0x3D);
        let cases = (0..400).map(|_| {
            let side = a_side(&mut rng);
            let which = rng.random_range(0..4u8);
            let mut arena = Arena::baseline();
            let combatant = arena.battle.side_mut(side);
            combatant.unmodified_stats = [(); 5].map(|_| a_stat(&mut rng));
            combatant.stat_mods = [(); 6].map(|_| a_stat_mod(&mut rng));
            let input = json!({"side": side, "arena": sparse(&arena), "which": which});
            (input, calculate_modified_stat(&mut oracle, &arena, side, which))
        }).collect();
        write("calculate_modified_stat", cases);
    }

    #[test]
    #[ignore = "a tool: writes pokered/fixtures/battle/apply_burn_and_paralysis_penalties.jsonl under GB_REGEN_FIXTURES=1"]
    fn harvest_apply_burn_and_paralysis_penalties() {
        let mut oracle = super::super::oracle();
        let mut rng = seeded(0xB42);
        let cases = (0..300).map(|_| {
            let side = a_side(&mut rng);
            let mut arena = Arena::baseline();
            for combatant in [&mut arena.battle.player, &mut arena.battle.enemy] {
                combatant.mon.status = a_status(&mut rng);
                combatant.mon.stats[1] = a_stat(&mut rng);
                combatant.mon.stats[3] = if rng.random_bool(0.2) { rng.random_range(0..=8) } else { a_stat(&mut rng) };
            }
            (input(Some(side), &arena), apply_burn_and_paralysis_penalties(&mut oracle, &arena, side))
        }).collect();
        write("apply_burn_and_paralysis_penalties", cases);
    }

    #[test]
    #[ignore = "a tool: writes pokered/fixtures/battle/apply_badge_stat_boosts.jsonl under GB_REGEN_FIXTURES=1"]
    fn harvest_apply_badge_stat_boosts() {
        let mut oracle = super::super::oracle();
        let mut rng = seeded(0xBAD);
        let cases = (0..300).map(|_| {
            let mut arena = Arena::baseline();
            arena.badges = rng.random();
            arena.battle.player.mon.stats = [(); 5].map(|_| a_stat(&mut rng));
            (input(None, &arena), apply_badge_stat_boosts(&mut oracle, &arena))
        }).collect();
        write("apply_badge_stat_boosts", cases);
    }

    #[test]
    #[ignore = "a tool: writes pokered/fixtures/battle/first_to_move.jsonl under GB_REGEN_FIXTURES=1"]
    fn harvest_first_to_move() {
        let mut oracle = super::super::oracle();
        let mut rng = seeded(0xF125);
        let special = [PokemonMoveName::QuickAttack as u8, PokemonMoveName::Counter as u8];
        let cases = (0..400).map(|_| {
            let mut arena = Arena::baseline();
            for combatant in [&mut arena.battle.player, &mut arena.battle.enemy] {
                combatant.selected_move = match rng.random_range(0..4) {
                    0 | 1 => pick(&mut rng, &special),
                    2 => 0xFF,
                    _ => a_move(&mut rng) as u8,
                };
            }
            let speed = a_stat(&mut rng);
            arena.battle.player.mon.stats[3] = speed;
            arena.battle.enemy.mon.stats[3] = if rng.random_bool(0.5) { speed } else { a_stat(&mut rng) };
            (input(None, &arena), first_to_move(&mut oracle, &arena))
        }).collect();
        write("first_to_move", cases);
    }

    /// The fields the damage calculation reads, at random, over the baseline.
    fn damage_arena(rng: &mut StdRng) -> Arena {
        let mut arena = Arena::baseline();
        let battle = &mut arena.battle;
        for combatant in [&mut battle.player, &mut battle.enemy] {
            let mon = &mut combatant.mon;
            mon.species = a_species(rng);
            mon.types = poke_core::base_stats::BaseStats::of(mon.species).types;
            mon.level = rng.random_range(1..=100);
            mon.stats = [(); 5].map(|_| a_safe_stat(rng));
            if rng.random_bool(0.2) {
                mon.status = rng.random_range(1..=7);
            }
            combatant.stat_mods[4] = a_stat_mod(rng);
            combatant.stat_mods[5] = a_stat_mod(rng);
            combatant.status1.set(Status1::INVULNERABLE, rng.random_bool(0.05));
            for (flag, chance) in [(Status2::PROTECTED_BY_MIST, 0.1), (Status2::USING_X_ACCURACY, 0.1),
                                   (Status2::GETTING_PUMPED, 0.1)] {
                combatant.status2.set(flag, rng.random_bool(chance));
            }
            combatant.status3 = Status3::from_bits_retain(if rng.random_bool(0.3) { rng.random_range(0..8) } else { 0 });
        }
        battle.enemy.mon.dvs = pokered::systems::stats::Dvs([rng.random(), rng.random()]);
        battle.damage = if rng.random_bool(0.5) { 0 } else { a_damage(rng) };
        arena.party[0].stats = [(); 5].map(|_| a_safe_stat(rng));
        arena
    }

    #[test]
    #[ignore = "a tool: writes pokered/fixtures/battle/calc_move_damage.jsonl under GB_REGEN_FIXTURES=1"]
    fn harvest_calc_move_damage() {
        let mut oracle = super::super::oracle();
        let mut rng = seeded(0xDA3A);
        let cases = (0..300).map(|_| {
            let side = a_side(&mut rng);
            let mut arena = damage_arena(&mut rng);
            let name = match rng.random_range(0..6) {
                0 => pick(&mut rng, &[PokemonMoveName::Counter, PokemonMoveName::SeismicToss, PokemonMoveName::SuperFang,
                    PokemonMoveName::Explosion, PokemonMoveName::Fissure, PokemonMoveName::DreamEater,
                    PokemonMoveName::Swift, PokemonMoveName::DoubleKick, PokemonMoveName::Slash]),
                _ => a_move(&mut rng),
            };
            using(&mut arena, side, name);
            let target = arena.battle.side_mut(side.other());
            let answer = a_move(&mut rng);
            target.current_move = MoveData::of_move(answer);
            target.selected_move = answer as u8;
            (input(Some(side), &arena), calc_move_damage(&mut oracle, &arena, side))
        }).collect();
        write("calc_move_damage", cases);
    }
}
