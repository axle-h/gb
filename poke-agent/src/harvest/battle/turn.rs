//! `pokered/fixtures/battle/`: the status checks before a move, what lands after it, and the
//! residual damage at the end of a turn.

use gb::ram::ROM;
use pokered::systems::battle::turn::MoveMenu;
use pokered::systems::battle::{Arena, Side};
use serde_json::{json, Value};
use crate::pokemon::symbols::{pokered_symbols as sym, DmgPointer};
use super::{changed, label_at, read_arena, run, text_printed, write_arena, Oracle, PRESENTATION};

/// Writes `arena` on `side`'s turn, runs `routine` with presentation skipped, and gives the run to
/// `finish` before the arena is read back.
fn on_turn(oracle: &mut Oracle, arena: &Arena, side: Side, routine: DmgPointer,
           finish: impl FnOnce(&mut Oracle) -> Value) -> (Value, Vec<u8>) {
    write_arena(oracle, arena);
    oracle.write(sym::hWhoseTurn, &[side as u8]);
    let mut texts = vec![];
    let ran = run(oracle, routine, &[], &PRESENTATION, &mut |name, gb| text_printed(&mut texts, name, gb));
    let returned = finish(oracle);
    let after = read_arena(oracle);
    let change = changed(&serde_json::to_value(arena).unwrap(), &serde_json::to_value(&after).unwrap())
        .unwrap_or(Value::Null);
    (json!({"changed": change, "returned": [returned, texts]}), ran.rng)
}

/// `CheckPlayerStatusConditions` or `CheckEnemyStatusConditions`, and where they send the turn. The
/// enemy's Bide swaps the levels for the continuation it returns to, which swaps them back.
fn check_status_conditions(oracle: &mut Oracle, arena: &Arena, side: Side) -> (Value, Vec<u8>) {
    let routine = match side {
        Side::Player => sym::CheckPlayerStatusConditions,
        Side::Enemy => sym::CheckEnemyStatusConditions,
    };
    on_turn(oracle, arena, side, routine, |oracle| {
        let registers = oracle.registers();
        if !registers.flags.z {
            return json!("Move");
        }
        let hl = u16::from_be_bytes([registers.h, registers.l]);
        let label = label_at(&oracle.gb, hl).unwrap_or_else(|| panic!("no label at {hl:#06x}"));
        if label == "HandleIfEnemyMoveMissed" {
            let (player, enemy) = (oracle.read(sym::wBattleMonLevel, 1)[0], oracle.read(sym::wEnemyMonLevel, 1)[0]);
            oracle.write(sym::wBattleMonLevel, &[enemy]);
            oracle.write(sym::wEnemyMonLevel, &[player]);
        }
        json!(match label {
            "ExecutePlayerMoveDone" | "ExecuteEnemyMoveDone" => "MoveDone",
            "HandleIfPlayerMoveMissed" | "HandleIfEnemyMoveMissed" => "HandleIfMoveMissed",
            "PlayerCalcMoveDamage" | "EnemyCalcMoveDamage" => "CalcMoveDamage",
            "GetPlayerAnimationType" | "GetEnemyAnimationType" => "GetAnimationType",
            "PlayerCanExecuteMove" | "EnemyCanExecuteMove" => "CanExecuteMove",
            other => panic!("continues at {other}"),
        })
    })
}

fn handle_poison_burn_leech_seed(oracle: &mut Oracle, arena: &Arena, side: Side) -> (Value, Vec<u8>) {
    on_turn(oracle, arena, side, sym::HandlePoisonBurnLeechSeed, |oracle| json!(oracle.registers().flags.z))
}

fn apply_attack_to_pokemon(oracle: &mut Oracle, arena: &Arena, side: Side) -> (Value, Vec<u8>) {
    let routine = match side {
        Side::Player => sym::ApplyAttackToEnemyPokemon,
        Side::Enemy => sym::ApplyAttackToPlayerPokemon,
    };
    on_turn(oracle, arena, side, routine, |_| Value::Null)
}

fn handle_building_rage(oracle: &mut Oracle, arena: &Arena, side: Side) -> (Value, Vec<u8>) {
    on_turn(oracle, arena, side, sym::HandleBuildingRage, |_| Value::Null)
}

fn mirror_move_copy_move(oracle: &mut Oracle, arena: &Arena, side: Side) -> (Value, Vec<u8>) {
    on_turn(oracle, arena, side, sym::MirrorMoveCopyMove, |oracle| json!(!oracle.registers().flags.z))
}

fn metronome_pick_move(oracle: &mut Oracle, arena: &Arena, side: Side) -> (Value, Vec<u8>) {
    on_turn(oracle, arena, side, sym::MetronomePickMove, |_| Value::Null)
}

/// `DecrementPP`, called as `Bankswitch` leaves it, with `de` on the player's selected move.
fn decrement_pp(oracle: &mut Oracle, arena: &Arena) -> (Value, Vec<u8>) {
    oracle.registers_mut().set_de(sym::wPlayerSelectedMove.address);
    on_turn(oracle, arena, Side::Player, sym::DecrementPP, |_| Value::Null)
}

fn check_num_attacks_left(oracle: &mut Oracle, arena: &Arena) -> (Value, Vec<u8>) {
    on_turn(oracle, arena, Side::Player, sym::CheckNumAttacksLeft, |_| Value::Null)
}

fn handle_self_confusion_damage(oracle: &mut Oracle, arena: &Arena) -> (Value, Vec<u8>) {
    on_turn(oracle, arena, Side::Player, sym::HandleSelfConfusionDamage, |_| Value::Null)
}

/// `CheckForDisobedience` from a move menu, and whether the mon obeys, as its `nz`, and where the
/// menu was left.
fn check_for_disobedience(oracle: &mut Oracle, arena: &Arena, menu: MoveMenu) -> (Value, Vec<u8>) {
    oracle.write(sym::wCurrentMenuItem, &[menu.current]);
    oracle.write(sym::wMaxMenuItem, &[menu.max]);
    on_turn(oracle, arena, Side::Player, sym::CheckForDisobedience, |oracle| {
        let current = oracle.gb.core().mmu().read(sym::wCurrentMenuItem.address);
        json!({"obeys": !oracle.registers().flags.z, "menu": MoveMenu { current, max: menu.max }})
    })
}

/// The oracle's own check: a mon asleep for one more turn wakes and does not move.
#[test]
fn a_mon_on_its_last_turn_of_sleep_wakes_up() {
    let mut oracle = super::oracle();
    let mut arena = Arena::baseline();
    arena.battle.player.mon.status = 1;
    let (output, _) = check_status_conditions(&mut oracle, &arena, Side::Player);
    assert_eq!(output["returned"], json!(["MoveDone", ["WokeUpText"]]));
    assert_eq!(output["changed"]["battle"]["player"]["mon"]["status"], json!(0));
}

#[cfg(feature = "slow-tests")]
mod harvest {
    use poke_core::move_name::PokemonMoveName;
    use poke_core::moves::MoveData;
    use pokered::systems::battle::{status, BattleKind, Status1, Status2, Status3};
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

    /// The conditions a turn checks, at random, for both sides.
    fn a_turn_arena(rng: &mut StdRng) -> Arena {
        let mut arena = Arena::baseline();
        let battle = &mut arena.battle;
        battle.damage = a_damage(rng);
        battle.kind = pick(rng, &[BattleKind::Wild, BattleKind::Trainer]);
        for combatant in [&mut battle.player, &mut battle.enemy] {
            combatant.mon.status = match rng.random_range(0..10) {
                0 => rng.random_range(1..=7),
                1 => status::FRZ,
                2 => status::PAR,
                3 => status::PSN,
                4 => status::BRN,
                5 => status::PSN | status::BRN,
                _ => 0,
            };
            for (flag, chance) in [(Status1::FLINCHED, 0.1), (Status1::CONFUSED, 0.25), (Status1::STORING_ENERGY, 0.1),
                                   (Status1::THRASHING_ABOUT, 0.1), (Status1::USING_TRAPPING_MOVE, 0.1),
                                   (Status1::CHARGING_UP, 0.1)] {
                combatant.status1.set(flag, rng.random_bool(chance));
            }
            for (flag, chance) in [(Status2::NEEDS_TO_RECHARGE, 0.1), (Status2::USING_RAGE, 0.1),
                                   (Status2::HAS_SUBSTITUTE_UP, 0.15), (Status2::SEEDED, 0.25)] {
                combatant.status2.set(flag, rng.random_bool(chance));
            }
            for (flag, chance) in [(Status3::BADLY_POISONED, 0.3), (Status3::HAS_REFLECT_UP, 0.2)] {
                combatant.status3.set(flag, rng.random_bool(chance));
            }
            combatant.confused_counter = rng.random_range(0..=5);
            combatant.num_attacks_left = rng.random_range(0..=4);
            combatant.toxic_counter = pick(rng, &[0, 1, 2, 7, 15, 255]);
            combatant.bide_accumulated_damage = if rng.random_bool(0.5) { 0 } else { a_damage(rng) };
            combatant.substitute_hp = rng.random();
            if rng.random_bool(0.3) {
                combatant.disabled_move = rng.random_range(1..=4) << 4 | rng.random_range(1..=3);
                combatant.disabled_move_number = a_move(rng) as u8;
            }
            let chosen = a_move(rng);
            combatant.selected_move = if rng.random_bool(0.2) { combatant.disabled_move_number.max(1) } else { chosen as u8 };
            combatant.current_move = MoveData::of_move(if rng.random_bool(0.1) { PokemonMoveName::Fly } else { chosen });
            let max = if rng.random_bool(0.2) { pick(rng, &[1, 15, 16, 17, 255, 1023, 1024]) } else { rng.random_range(20..=999) };
            combatant.mon.stats = [max, a_safe_stat(rng), a_safe_stat(rng), a_safe_stat(rng), a_safe_stat(rng)];
            combatant.mon.hp = if rng.random_bool(0.2) { rng.random_range(0..=4) } else { rng.random_range(0..=max) };
            combatant.mon.level = rng.random_range(2..=100);
        }
        arena.party[0].stats = [(); 5].map(|_| a_safe_stat(rng));
        arena
    }

    /// `count` cases of `routine`, each from a random turn arena or, where the routine reads little,
    /// from the baseline for `call` to fill in.
    fn harvest_turn(routine: &str, seed: u64, count: usize, from_turn_arena: bool,
                    call: impl Fn(&mut Oracle, &mut StdRng, Arena, Side) -> (Value, (Value, Vec<u8>))) {
        let mut oracle = super::super::oracle();
        let mut rng = seeded(seed);
        let cases = (0..count).map(|_| {
            let side = a_side(&mut rng);
            let arena = if from_turn_arena { a_turn_arena(&mut rng) } else { Arena::baseline() };
            call(&mut oracle, &mut rng, arena, side)
        }).collect();
        write(routine, cases);
    }

    fn input(side: Side, arena: &Arena) -> Value {
        json!({"side": side, "arena": sparse(arena)})
    }

    #[test]
    #[ignore = "a tool: writes pokered/fixtures/battle/check_status_conditions.jsonl under GB_REGEN_FIXTURES=1"]
    fn harvest_check_status_conditions() {
        harvest_turn("check_status_conditions", 0xC5C, 500, true, |oracle, rng, mut arena, side| {
            if rng.random_bool(0.15) {
                let other = arena.battle.side_mut(side.other());
                other.status1.remove(Status1::USING_TRAPPING_MOVE);
                let me = arena.battle.side_mut(side);
                me.mon.status = 0;
                me.status1 = Status1::STORING_ENERGY;
                me.status2.remove(Status2::NEEDS_TO_RECHARGE);
                me.num_attacks_left = rng.random_range(1..=2);
                me.disabled_move_number = 0;
            }
            (input(side, &arena), check_status_conditions(oracle, &arena, side))
        });
    }

    #[test]
    #[ignore = "a tool: writes pokered/fixtures/battle/handle_poison_burn_leech_seed.jsonl under GB_REGEN_FIXTURES=1"]
    fn harvest_handle_poison_burn_leech_seed() {
        harvest_turn("handle_poison_burn_leech_seed", 0x9B1, 250, false, |oracle, rng, mut arena, side| {
            for combatant in [&mut arena.battle.player, &mut arena.battle.enemy] {
                combatant.mon.status = pick(rng, &[0, status::PSN, status::BRN, status::PSN | status::BRN, status::PAR]);
                combatant.status2.set(Status2::SEEDED, rng.random_bool(0.4));
                combatant.status3.set(Status3::BADLY_POISONED, rng.random_bool(0.3));
                combatant.toxic_counter = pick(rng, &[0, 1, 2, 7, 15, 255]);
                let max = if rng.random_bool(0.3) { pick(rng, &[1, 15, 16, 17, 255, 1023, 1024, 2047]) } else { rng.random_range(20..=999) };
                combatant.mon.stats[0] = max;
                combatant.mon.hp = if rng.random_bool(0.3) { rng.random_range(0..=4) } else { rng.random_range(0..=max) };
            }
            (input(side, &arena), handle_poison_burn_leech_seed(oracle, &arena, side))
        });
    }

    #[test]
    #[ignore = "a tool: writes pokered/fixtures/battle/apply_attack_to_pokemon.jsonl under GB_REGEN_FIXTURES=1"]
    fn harvest_apply_attack_to_pokemon() {
        let specials = [PokemonMoveName::SeismicToss, PokemonMoveName::NightShade, PokemonMoveName::Sonicboom,
            PokemonMoveName::DragonRage, PokemonMoveName::Psywave, PokemonMoveName::Psywave, PokemonMoveName::SuperFang,
            PokemonMoveName::Fissure, PokemonMoveName::Tackle, PokemonMoveName::Growl];
        harvest_turn("apply_attack_to_pokemon", 0xA77, 300, false, |oracle, rng, mut arena, side| {
            let name = pick(rng, &specials);
            using(&mut arena, side, name);
            arena.battle.damage = a_damage(rng);
            for combatant in [&mut arena.battle.player, &mut arena.battle.enemy] {
                combatant.mon.level = rng.random_range(2..=100);
                combatant.mon.hp = if rng.random_bool(0.2) { rng.random_range(0..=3) } else { rng.random_range(0..=999) };
                combatant.status2.set(Status2::HAS_SUBSTITUTE_UP, rng.random_bool(0.2));
                combatant.substitute_hp = rng.random();
            }
            // Psywave draws for ever where one and a half times the level is 0 in a byte, at 171, and at
            // level 1 on the player's side, where it must also draw above 0.
            let user = arena.battle.side_mut(side);
            user.mon.level = user.mon.level.max(2);
            if rng.random_bool(0.3) {
                arena.battle.side_mut(side).mon.level = pick(rng, &[2, 3, 100, 170, 172, 255]);
            }
            (input(side, &arena), apply_attack_to_pokemon(oracle, &arena, side))
        });
    }

    #[test]
    #[ignore = "a tool: writes pokered/fixtures/battle/handle_building_rage.jsonl under GB_REGEN_FIXTURES=1"]
    fn harvest_handle_building_rage() {
        harvest_turn("handle_building_rage", 0x2A6E, 120, false, |oracle, rng, mut arena, side| {
            arena.badges = rng.random();
            let raging = arena.battle.side_mut(side.other());
            raging.status2.set(Status2::USING_RAGE, rng.random_bool(0.8));
            raging.mon.stats[1] = if rng.random_bool(0.2) { 999 } else { a_safe_stat(rng) };
            raging.stat_mods[0] = a_stat_mod(rng);
            raging.unmodified_stats = [(); 5].map(|_| a_safe_stat(rng));
            (input(side, &arena), handle_building_rage(oracle, &arena, side))
        });
    }

    #[test]
    #[ignore = "a tool: writes pokered/fixtures/battle/mirror_move_copy_move.jsonl under GB_REGEN_FIXTURES=1"]
    fn harvest_mirror_move_copy_move() {
        harvest_turn("mirror_move_copy_move", 0x3140, 120, false, |oracle, rng, mut arena, side| {
            arena.battle.side_mut(side.other()).used_move = match rng.random_range(0..4) {
                0 => 0,
                1 => PokemonMoveName::MirrorMove as u8,
                _ => a_move(rng) as u8,
            };
            let user = arena.battle.side_mut(side);
            user.move_list_index = rng.random_range(0..4);
            user.mon.pp = [(); 4].map(|_| rng.random());
            arena.party[0].mon.pp = [(); 4].map(|_| rng.random());
            (input(side, &arena), mirror_move_copy_move(oracle, &arena, side))
        });
    }

    #[test]
    #[ignore = "a tool: writes pokered/fixtures/battle/metronome_pick_move.jsonl under GB_REGEN_FIXTURES=1"]
    fn harvest_metronome_pick_move() {
        harvest_turn("metronome_pick_move", 0x3E7, 120, false, |oracle, rng, mut arena, side| {
            let user = arena.battle.side_mut(side);
            user.move_list_index = rng.random_range(0..4);
            user.mon.pp = [(); 4].map(|_| rng.random());
            (input(side, &arena), metronome_pick_move(oracle, &arena, side))
        });
    }

    #[test]
    #[ignore = "a tool: writes pokered/fixtures/battle/decrement_pp.jsonl under GB_REGEN_FIXTURES=1"]
    fn harvest_decrement_pp() {
        harvest_turn("decrement_pp", 0xDEC, 120, false, |oracle, rng, mut arena, _| {
            let player = &mut arena.battle.player;
            player.move_list_index = rng.random_range(0..4);
            player.mon.pp = [(); 4].map(|_| rng.random());
            if rng.random_bool(0.1) {
                player.selected_move = PokemonMoveName::Struggle as u8;
            }
            player.status3.set(Status3::TRANSFORMED, rng.random_bool(0.3));
            player.status1.set(Status1::ATTACKING_MULTIPLE_TIMES, rng.random_bool(0.1));
            arena.party[0].mon.pp = [(); 4].map(|_| rng.random());
            (input(Side::Player, &arena), decrement_pp(oracle, &arena))
        });
    }

    #[test]
    #[ignore = "a tool: writes pokered/fixtures/battle/check_num_attacks_left.jsonl under GB_REGEN_FIXTURES=1"]
    fn harvest_check_num_attacks_left() {
        harvest_turn("check_num_attacks_left", 0xC4A, 100, false, |oracle, rng, mut arena, _| {
            for combatant in [&mut arena.battle.player, &mut arena.battle.enemy] {
                combatant.num_attacks_left = rng.random_range(0..=2);
                combatant.status1.set(Status1::USING_TRAPPING_MOVE, rng.random_bool(0.5));
            }
            (input(Side::Player, &arena), check_num_attacks_left(oracle, &arena))
        });
    }

    #[test]
    #[ignore = "a tool: writes pokered/fixtures/battle/handle_self_confusion_damage.jsonl under GB_REGEN_FIXTURES=1"]
    fn harvest_handle_self_confusion_damage() {
        harvest_turn("handle_self_confusion_damage", 0x5CD, 120, true, |oracle, _, arena, _| {
            (input(Side::Player, &arena), handle_self_confusion_damage(oracle, &arena))
        });
    }

    #[test]
    #[ignore = "a tool: writes pokered/fixtures/battle/check_for_disobedience.jsonl under GB_REGEN_FIXTURES=1"]
    fn harvest_check_for_disobedience() {
        const PLAYER_ID: u16 = 0x0BAD;
        harvest_turn("check_for_disobedience", 0xD15, 300, false, |oracle, rng, mut arena, _| {
            arena.player_id = PLAYER_ID;
            let any: u8 = rng.random();
            arena.badges = pick(rng, &[0, 0b10, 0b1010, 0b101010, 0xFF, any]);
            arena.party[0].mon.ot_id = if rng.random_bool(0.85) { rng.random() } else { PLAYER_ID };
            let player = &mut arena.battle.player;
            player.mon.level = if rng.random_bool(0.5) { rng.random_range(40..=100) } else { rng.random_range(2..=100) };
            let len = rng.random_range(1..=4);
            player.mon.moves = [0, 1, 2, 3].map(|slot| (slot < len).then(|| a_move(rng)));
            // A move with a PP byte besides the chosen one, or the random pick never ends.
            player.mon.pp = [0, 1, 2, 3].map(|slot| if slot < len { rng.random_range(0..=0x3F) } else { 0 });
            let current = rng.random_range(0..len) as u8;
            if len > 1 && (0..len).all(|slot| slot == current as usize || player.mon.pp[slot] == 0) {
                player.mon.pp[(current as usize + 1) % len] = 1;
            }
            if rng.random_bool(0.1) {
                player.selected_move = PokemonMoveName::Struggle as u8;
            }
            let menu = MoveMenu { current, max: len as u8 + 1 };
            (json!({"arena": sparse(&arena), "menu": menu}), check_for_disobedience(oracle, &arena, menu))
        });
    }
}
