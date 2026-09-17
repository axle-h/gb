//! `pokered/fixtures/battle/`: the move effects, entered at their own labels with `hWhoseTurn` set.

use gb::ram::RAM;
use pokered::systems::battle::effects::MimicMenu;
use pokered::systems::battle::{Arena, Side};
use serde_json::{json, Value};
use crate::pokemon::symbols::{pokered_symbols as sym, DmgPointer};
use super::{run_arena_observed, text_printed, Oracle, PRESENTATION};

/// `routine` on `side`'s turn: what it changed, the texts it printed, and its random bytes.
pub(super) fn effect(oracle: &mut Oracle, arena: &Arena, side: Side, routine: DmgPointer) -> (Value, Vec<u8>) {
    jump_move_effect(oracle, arena, side, routine, MimicMenu { cursor: 0, chosen: 0 })
}

/// `routine`, and for the player's Mimic the answers its move menu is given.
fn jump_move_effect(oracle: &mut Oracle, arena: &Arena, side: Side, routine: DmgPointer, menu: MimicMenu) -> (Value, Vec<u8>) {
    oracle.write(sym::hWhoseTurn, &[side as u8]);
    oracle.write(sym::wCurrentMenuItem, &[menu.cursor]);
    let mut skips = PRESENTATION.to_vec();
    skips.extend([
        ("MoveSelectionMenu", sym::MoveSelectionMenu),
        ("LoadScreenTilesFromBuffer1", sym::LoadScreenTilesFromBuffer1),
        ("AnimationSubstitute", sym::AnimationSubstitute),
        ("AnimationTransformMon", sym::AnimationTransformMon),
    ]);
    let mut texts = vec![];
    let (_, changed, ran) = run_arena_observed(oracle, arena, routine, &[], &skips, &mut |name, gb| {
        text_printed(&mut texts, name, gb);
        if name == "MoveSelectionMenu" {
            gb.core_mut().mmu_mut().write(sym::wCurrentMenuItem.address, menu.chosen);
        }
    });
    (json!({"changed": changed, "returned": texts}), ran.rng)
}

/// The oracle's own check: Swords Dance from +5 stops at +6.
#[test]
fn swords_dance_stops_at_plus_six() {
    let mut oracle = super::oracle();
    let mut arena = Arena::baseline();
    arena.battle.player.current_move = poke_core::moves::MoveData::of_move(poke_core::move_name::PokemonMoveName::SwordsDance);
    arena.battle.player.stat_mods[0] = 12;
    let (output, _) = effect(&mut oracle, &arena, Side::Player, sym::StatModifierUpEffect);
    assert_eq!(output["changed"]["battle"]["player"]["stat_mods"]["0"], json!(13));
    assert_eq!(output["returned"], json!(["MonsStatsRoseText"]));
}

#[cfg(feature = "slow-tests")]
mod harvest {
    use poke_core::move_name::PokemonMoveName;
    use poke_core::moves::MoveData;
    use pokered::systems::battle::{effect, status, BattleKind, Status1, Status2, Status3};
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

    /// The fields a stat modifier effect reads, at random.
    fn stat_arena(rng: &mut StdRng, side: Side, effects: &[u8]) -> Arena {
        let mut arena = Arena::baseline();
        arena.badges = rng.random();
        let battle = &mut arena.battle;
        let user = battle.side_mut(side);
        user.current_move.effect = pick(rng, effects);
        user.current_move.animation = if rng.random_bool(0.1) { PokemonMoveName::Minimize as u8 } else { rng.random_range(1..=165) };
        user.current_move.accuracy = rng.random();
        battle.move_didnt_miss = rng.random_bool(0.3);
        for combatant in [&mut battle.player, &mut battle.enemy] {
            combatant.stat_mods = [(); 6].map(|_| a_stat_mod(rng));
            combatant.unmodified_stats = [(); 5].map(|_| a_stat(rng));
            combatant.mon.stats = [(); 5].map(|_| match rng.random_range(0..6) {
                0 => 999,
                1 => 1,
                _ => a_stat(rng),
            });
            combatant.mon.status = if rng.random_bool(0.3) { pick(rng, &[status::PAR, status::BRN, status::PAR | status::BRN]) } else { 0 };
            combatant.status1.set(Status1::INVULNERABLE, rng.random_bool(0.1));
            for (flag, chance) in [(Status2::HAS_SUBSTITUTE_UP, 0.15), (Status2::PROTECTED_BY_MIST, 0.1),
                                   (Status2::USING_X_ACCURACY, 0.1)] {
                combatant.status2.set(flag, rng.random_bool(chance));
            }
        }
        arena
    }

    #[test]
    #[ignore = "a tool: writes pokered/fixtures/battle/stat_modifier_up_effect.jsonl under GB_REGEN_FIXTURES=1"]
    fn harvest_stat_modifier_up_effect() {
        let mut oracle = super::super::oracle();
        let mut rng = seeded(0x5A7);
        let effects: Vec<u8> = (effect::ATTACK_UP1_EFFECT..=effect::EVASION_UP1_EFFECT)
            .chain(effect::ATTACK_UP2_EFFECT..=effect::EVASION_UP2_EFFECT).collect();
        let cases = (0..400).map(|_| {
            let side = a_side(&mut rng);
            let arena = stat_arena(&mut rng, side, &effects);
            (json!({"side": side, "arena": sparse(&arena)}), effect(&mut oracle, &arena, side, sym::StatModifierUpEffect))
        }).collect();
        write("stat_modifier_up_effect", cases);
    }

    #[test]
    #[ignore = "a tool: writes pokered/fixtures/battle/stat_modifier_down_effect.jsonl under GB_REGEN_FIXTURES=1"]
    fn harvest_stat_modifier_down_effect() {
        let mut oracle = super::super::oracle();
        let mut rng = seeded(0x5D0);
        let effects: Vec<u8> = (effect::ATTACK_DOWN1_EFFECT..=effect::EVASION_DOWN1_EFFECT)
            .chain(effect::ATTACK_DOWN2_EFFECT..=effect::EVASION_DOWN2_EFFECT)
            .chain(effect::ATTACK_DOWN_SIDE_EFFECT..=effect::SPECIAL_DOWN_SIDE_EFFECT).collect();
        let cases = (0..400).map(|_| {
            let side = a_side(&mut rng);
            let arena = stat_arena(&mut rng, side, &effects);
            (json!({"side": side, "arena": sparse(&arena)}), effect(&mut oracle, &arena, side, sym::StatModifierDownEffect))
        }).collect();
        write("stat_modifier_down_effect", cases);
    }

    /// The moves whose effect is one of `effects`.
    fn moves_with(effects: &[u8]) -> Vec<PokemonMoveName> {
        (1..=PokemonMoveName::Struggle as u8).filter_map(PokemonMoveName::from_repr)
            .filter(|&name| effects.contains(&MoveData::of_move(name).effect))
            .collect()
    }

    /// What an effect reads, beyond the move being used.
    #[derive(Clone, Copy)]
    struct Reads {
        levels: bool,
        hp: bool,
        types: bool,
        moves: bool,
        stats: bool,
    }

    const NOTHING: Reads = Reads { levels: false, hp: false, types: false, moves: false, stats: false };

    /// A state for a move with one of `effects` to be used in, randomising what `reads` names and
    /// always the statuses, the flags and what a hit test reads.
    fn an_effect_arena(rng: &mut StdRng, side: Side, effects: &[u8], reads: Reads) -> Arena {
        let mut arena = Arena::baseline();
        let battle = &mut arena.battle;
        battle.kind = pick(rng, &[BattleKind::Wild, BattleKind::Wild, BattleKind::Trainer, BattleKind::Lost]);
        battle.damage = a_damage(rng);
        battle.move_didnt_miss = rng.random_bool(0.3);
        for combatant in [&mut battle.player, &mut battle.enemy] {
            let mon = &mut combatant.mon;
            if reads.levels {
                mon.level = if rng.random_bool(0.3) { rng.random_range(1..=20) } else { rng.random_range(1..=100) };
            }
            if reads.hp {
                let max = match rng.random_range(0..4) {
                    0 => rng.random_range(1..=20),
                    1 => pick(rng, &[255, 256, 511, 512, 767, 1023]),
                    _ => rng.random_range(20..=999),
                };
                mon.stats[0] = max;
                mon.hp = match rng.random_range(0..8) {
                    0 => max,
                    1 => max.saturating_sub(255),
                    2 => max.saturating_sub(511),
                    3 => 0,
                    4 => max / 4,
                    5 => (max / 4).saturating_sub(1),
                    _ => rng.random_range(0..=max),
                };
            }
            if reads.types {
                mon.types = [a_type(rng), a_type(rng)];
            }
            if reads.moves {
                let len = rng.random_range(1..=4);
                mon.moves = [0, 1, 2, 3].map(|slot| (slot < len).then(|| a_move(rng)));
                // An empty slot has no PP: Disable draws forever for a move with PP where only an empty slot has any.
                mon.pp = [0, 1, 2, 3].map(|slot| match slot < len {
                    false => 0,
                    true if rng.random_bool(0.3) => pick(rng, &[0, 0x40, 0xC0, 1]),
                    true => rng.random(),
                });
                combatant.move_list_index = rng.random_range(0..4);
            }
            if reads.stats {
                mon.species = a_species(rng);
                mon.catch_rate = rng.random();
                mon.dvs = pokered::systems::stats::Dvs([rng.random(), rng.random()]);
                mon.stats[1..].iter_mut().for_each(|stat| *stat = a_safe_stat(rng));
                combatant.unmodified_stats = [(); 5].map(|_| a_safe_stat(rng));
                combatant.stat_mods = [(); 6].map(|_| a_stat_mod(rng));
            }
            combatant.mon.status = match rng.random_range(0..8) {
                0..=3 => 0,
                4 => rng.random_range(1..=7),
                5 => status::FRZ,
                _ => pick(rng, &[status::PSN, status::BRN, status::PAR]),
            };
            combatant.stat_mods[4] = a_stat_mod(rng);
            combatant.stat_mods[5] = a_stat_mod(rng);
            for (flag, chance) in [(Status1::INVULNERABLE, 0.1), (Status1::CONFUSED, 0.2), (Status1::USING_TRAPPING_MOVE, 0.15),
                                   (Status1::ATTACKING_MULTIPLE_TIMES, 0.15)] {
                combatant.status1.set(flag, rng.random_bool(chance));
            }
            for (flag, chance) in [(Status2::HAS_SUBSTITUTE_UP, 0.15), (Status2::PROTECTED_BY_MIST, 0.15),
                                   (Status2::USING_X_ACCURACY, 0.1), (Status2::GETTING_PUMPED, 0.15),
                                   (Status2::NEEDS_TO_RECHARGE, 0.15), (Status2::SEEDED, 0.15)] {
                combatant.status2.set(flag, rng.random_bool(chance));
            }
            for (flag, chance) in [(Status3::BADLY_POISONED, 0.15), (Status3::HAS_LIGHT_SCREEN_UP, 0.2),
                                   (Status3::HAS_REFLECT_UP, 0.2), (Status3::TRANSFORMED, 0.1)] {
                combatant.status3.set(flag, rng.random_bool(chance));
            }
            if rng.random_bool(0.2) {
                combatant.disabled_move = rng.random_range(1..=4) << 4 | rng.random_range(1..=8);
            }
        }
        if effects.contains(&effect::PAY_DAY_EFFECT) {
            battle.total_pay_day_money = pick(rng, &[[0, 0, 0], [0, 0x12, 0x34], [0x99, 0x99, 0x90]]);
        }
        let name = pick(rng, &moves_with(effects));
        using(&mut arena, side, name);
        if rng.random_bool(0.3) {
            arena.battle.side_mut(side).current_move.accuracy = rng.random();
        }
        arena
    }

    fn harvest_effect(routine: &str, seed: u64, count: usize, effects: &[u8], reads: Reads) {
        let mut oracle = super::super::oracle();
        let mut rng = seeded(seed);
        let cases = (0..count).map(|_| {
            let side = a_side(&mut rng);
            let arena = an_effect_arena(&mut rng, side, effects, reads);
            let menu = MimicMenu { cursor: rng.random_range(0..4), chosen: rng.random_range(0..4) };
            let mut input = json!({"side": side, "arena": sparse(&arena)});
            if effects.contains(&effect::MIMIC_EFFECT) {
                input["menu"] = json!(menu);
            }
            (input, jump_move_effect(&mut oracle, &arena, side, sym::JumpMoveEffect, menu))
        }).collect();
        write(routine, cases);
    }

    macro_rules! effect_harvests {
        ($($name:ident: $seed:literal, $count:literal, [$($effect:ident),+], $reads:expr;)+) => {$(
            #[test]
            #[ignore = "a tool: writes a move effect's fixture under GB_REGEN_FIXTURES=1"]
            fn $name() {
                let routine = stringify!($name).trim_start_matches("harvest_");
                harvest_effect(routine, $seed, $count, &[$(effect::$effect),+], $reads);
            }
        )+};
    }

    effect_harvests! {
        harvest_sleep_effect: 0x51, 150, [SLEEP_EFFECT], NOTHING;
        harvest_poison_effect: 0x52, 200, [POISON_SIDE_EFFECT1, POISON_SIDE_EFFECT2, POISON_EFFECT], Reads { types: true, ..NOTHING };
        harvest_drain_hp_effect: 0x53, 100, [DRAIN_HP_EFFECT, DREAM_EATER_EFFECT], Reads { hp: true, ..NOTHING };
        harvest_freeze_burn_paralyze_effect: 0x54, 250, [BURN_SIDE_EFFECT1, FREEZE_SIDE_EFFECT1, PARALYZE_SIDE_EFFECT1,
            BURN_SIDE_EFFECT2, PARALYZE_SIDE_EFFECT2], Reads { types: true, stats: true, ..NOTHING };
        harvest_explode_effect: 0x55, 100, [EXPLODE_EFFECT], Reads { hp: true, ..NOTHING };
        harvest_bide_effect: 0x56, 100, [BIDE_EFFECT], NOTHING;
        harvest_thrash_petal_dance_effect: 0x57, 100, [THRASH_PETAL_DANCE_EFFECT], NOTHING;
        harvest_switch_and_teleport_effect: 0x58, 200, [SWITCH_AND_TELEPORT_EFFECT], Reads { levels: true, hp: true, ..NOTHING };
        harvest_two_to_five_attacks_effect: 0x59, 120, [TWO_TO_FIVE_ATTACKS_EFFECT, ATTACK_TWICE_EFFECT, TWINEEDLE_EFFECT], NOTHING;
        harvest_flinch_side_effect: 0x5A, 150, [FLINCH_SIDE_EFFECT1, FLINCH_SIDE_EFFECT2], NOTHING;
        harvest_charge_effect: 0x5B, 100, [CHARGE_EFFECT, FLY_EFFECT], NOTHING;
        harvest_trapping_effect: 0x5C, 100, [TRAPPING_EFFECT], NOTHING;
        harvest_mist_effect: 0x5D, 100, [MIST_EFFECT], NOTHING;
        harvest_focus_energy_effect: 0x5E, 100, [FOCUS_ENERGY_EFFECT], NOTHING;
        harvest_recoil_effect: 0x5F, 100, [RECOIL_EFFECT], Reads { hp: true, ..NOTHING };
        harvest_confusion_effect: 0x60, 150, [CONFUSION_EFFECT, CONFUSION_SIDE_EFFECT], NOTHING;
        harvest_heal_effect: 0x61, 150, [HEAL_EFFECT], Reads { hp: true, ..NOTHING };
        harvest_transform_effect: 0x62, 100, [TRANSFORM_EFFECT], Reads { moves: true, stats: true, types: true, ..NOTHING };
        harvest_reflect_light_screen_effect: 0x63, 100, [LIGHT_SCREEN_EFFECT, REFLECT_EFFECT], NOTHING;
        harvest_paralyze_effect: 0x64, 150, [PARALYZE_EFFECT], Reads { types: true, stats: true, ..NOTHING };
        harvest_substitute_effect: 0x65, 100, [SUBSTITUTE_EFFECT], Reads { hp: true, ..NOTHING };
        harvest_hyper_beam_effect: 0x66, 100, [HYPER_BEAM_EFFECT], NOTHING;
        harvest_rage_effect: 0x67, 100, [RAGE_EFFECT], NOTHING;
        harvest_mimic_effect: 0x68, 150, [MIMIC_EFFECT], Reads { moves: true, ..NOTHING };
        harvest_leech_seed_effect: 0x69, 100, [LEECH_SEED_EFFECT], Reads { types: true, ..NOTHING };
        harvest_splash_effect: 0x6A, 100, [SPLASH_EFFECT], NOTHING;
        harvest_disable_effect: 0x6B, 150, [DISABLE_EFFECT], Reads { moves: true, ..NOTHING };
        harvest_pay_day_effect: 0x6C, 100, [PAY_DAY_EFFECT], Reads { levels: true, ..NOTHING };
        harvest_conversion_effect: 0x6D, 100, [CONVERSION_EFFECT], Reads { types: true, ..NOTHING };
        harvest_haze_effect: 0x6E, 100, [HAZE_EFFECT], Reads { stats: true, ..NOTHING };
    }
}
