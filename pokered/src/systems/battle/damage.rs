//! `CriticalHitTest`, `HandleCounterMove`, `GetDamageVarsForPlayerAttack` and its enemy twin,
//! `CalculateDamage`, `AdjustDamageForMoveType`, `AIGetTypeEffectiveness`, `RandomizeDamage` and
//! `OneHitKOEffect_`: a damaging move's number, in the order `PlayerCalcMoveDamage` takes them.

use poke_core::base_stats::BaseStats;
use poke_core::battle_data::high_critical_moves;
use poke_core::move_name::PokemonMoveName;
use poke_core::types::matchups;
use serde::{Deserialize, Serialize};
use crate::party::PartyMon;
use crate::rng::Rng;
use crate::systems::math::{divide, multiply};
use crate::systems::stats::{calc_stat, Stat, MAX_STAT_VALUE};
use super::accuracy::move_hit_test;
use super::{effect, stat, Battle, CriticalHitOrOhko, Side, Status2, Status3, MAX_NEUTRAL_DAMAGE,
            MIN_NEUTRAL_DAMAGE, SPECIAL};

/// `b`, `c`, `d` and `e` as `GetDamageVarsFor*Attack` leaves them for `CalculateDamage`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DamageVars {
    pub attack: u8,
    pub defense: u8,
    pub power: u8,
    pub level: u8,
}

/// `CriticalHitTest`: half the attacker's base speed, doubled, and doubled again under Focus
/// Energy; then doubled twice for a high-critical move and halved for any other. A move with no
/// power draws nothing.
pub fn critical_hit_test(battle: &mut Battle, attacker: Side, rng: &mut impl Rng) {
    battle.critical_hit_or_ohko = CriticalHitOrOhko::Normal;
    let user = battle.side(attacker);
    let mut rate = BaseStats::of(user.mon.species).stats[stat::SPEED] >> 1;
    if user.current_move.power == 0 {
        return;
    }
    let doubled = |rate: u8| rate.checked_mul(2).unwrap_or(0xFF);
    rate = match (user.status2.contains(Status2::GETTING_PUMPED), battle.cartridge_bugs) {
        (false, _) => doubled(rate),
        // The cartridge shifts right under Focus Energy, a quarter of the unfocused rate.
        (true, true) => rate >> 1,
        (true, false) => doubled(doubled(rate)),
    };
    rate = if high_critical_moves().contains(&user.current_move.animation) { doubled(doubled(rate)) } else { rate >> 1 };
    if rng.random().rotate_left(3) < rate {
        battle.critical_hit_or_ohko = CriticalHitOrOhko::CriticalHit;
    }
}

/// What `HandleCounterMove` leaves in the zero flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Counter {
    /// Not Counter: the damage is calculated as usual.
    NotCounter,
    /// Counter, hit or missed: the damage is settled and the calculation skipped.
    Resolved,
}

pub(super) const NORMAL: u8 = 0;
pub(super) const FIGHTING: u8 = 1;

/// `HandleCounterMove`: twice the damage the target's Normal or Fighting move did to the Counter
/// user this turn, and then the usual hit test.
pub fn handle_counter_move(battle: &mut Battle, attacker: Side, rng: &mut impl Rng) -> Counter {
    let counter = PokemonMoveName::Counter as u8;
    if battle.side(attacker).selected_move != counter {
        return Counter::NotCounter;
    }
    battle.move_missed = true;
    let target = battle.side(attacker.other());
    if !battle.cartridge_bugs {
        if target.counter_damage == 0 {
            return Counter::Resolved;
        }
        battle.damage = target.counter_damage.saturating_mul(2);
        battle.move_missed = false;
        move_hit_test(battle, attacker, rng);
        return Counter::Resolved;
    }
    // The cartridge reads the move the target last selected, which the menu cursor sets, and
    // doubles `wDamage` whoever dealt it.
    if target.selected_move == counter || target.current_move.power == 0
        || !matches!(target.current_move.move_type, NORMAL | FIGHTING) || battle.damage == 0 {
        return Counter::Resolved;
    }
    battle.damage = battle.damage.saturating_mul(2);
    battle.move_missed = false;
    move_hit_test(battle, attacker, rng);
    Counter::Resolved
}

/// `GetDamageVarsForPlayerAttack` and `GetDamageVarsForEnemyAttack`: `None` for a move with no
/// power, having zeroed `wDamage` either way. Reflect and Light Screen double the defense, up to
/// 999; a critical hit reads both mons' unmodified stats instead, which drops stat modifiers, badge
/// boosts and the screens alike. A stat over a byte scales both by four, neither below 1.
pub fn get_damage_vars(battle: &mut Battle, party: &[PartyMon], attacker: Side) -> Option<DamageVars> {
    battle.damage = 0;
    let user = battle.side(attacker);
    let target = battle.side(attacker.other());
    let power = user.current_move.power;
    if power == 0 {
        return None;
    }
    let (offense, defense, screen) = if user.current_move.move_type < SPECIAL {
        (stat::ATTACK, stat::DEFENSE, Status3::HAS_REFLECT_UP)
    } else {
        (stat::SPECIAL, stat::SPECIAL, Status3::HAS_LIGHT_SCREEN_UP)
    };
    let bugs = battle.cartridge_bugs;
    let mut defense_stat = target.mon.stats[defense];
    if target.status3.contains(screen) {
        defense_stat = defense_stat.wrapping_shl(1);
        // The cartridge has no cap, and a doubled stat of 1024 or more wraps in the scaling.
        if !bugs {
            defense_stat = defense_stat.min(MAX_STAT_VALUE);
        }
    }
    let mut attack_stat = user.mon.stats[offense];
    let critical = battle.critical_hit_or_ohko != CriticalHitOrOhko::Normal;
    if critical && !bugs {
        (attack_stat, defense_stat) = (user.unmodified_stats[offense], target.unmodified_stats[defense]);
    } else if critical {
        // The cartridge reads the player's party stat, from before any Transform.
        let party_stat = |index: usize| party[battle.player_mon_number as usize].stats[index];
        (attack_stat, defense_stat) = match attacker {
            Side::Player => (party_stat(offense), get_enemy_mon_stat(battle, defense)),
            Side::Enemy => (get_enemy_mon_stat(battle, offense), party_stat(defense)),
        };
    }
    if (attack_stat | defense_stat) > 0xFF {
        defense_stat >>= 2;
        attack_stat = (attack_stat >> 2).max(1);
        // The cartridge lets a defense of 1 to 3 scale to 0, and hangs dividing by it.
        if !bugs {
            defense_stat = defense_stat.max(1);
        }
    }
    let mut level = user.mon.level;
    if critical {
        level = level.wrapping_shl(1);
    }
    Some(DamageVars { attack: attack_stat as u8, defense: defense_stat as u8, power, level })
}

/// `GetEnemyMonStat` outside a link battle: the enemy's stat worked out from its base, DVs and
/// level, with no stat experience.
fn get_enemy_mon_stat(battle: &Battle, index: usize) -> u16 {
    const STATS: [Stat; 5] = [Stat::Hp, Stat::Attack, Stat::Defense, Stat::Speed, Stat::Special];
    let enemy = &battle.enemy.mon;
    calc_stat(STATS[index], BaseStats::of(enemy.species).stats[index], enemy.dvs, None, enemy.level)
}

/// What `CalculateDamage` leaves in the zero flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Calculated {
    /// `nz`: go on to the type adjustment and the random factor.
    Damage,
    /// `z`: a move with no power, or a one-hit KO that missed; skip to the effects.
    Skipped,
}

/// `CalculateDamage`: `(2 * level / 5 + 2) * power * attack / defense / 50`, added to `wDamage`,
/// capped at 997 and then raised by 2. Explosion halves the defense, never below 1; the multi-hit
/// effects go on without power, and a one-hit KO runs its effect instead. A defense of 0 divides
/// by zero, which the cartridge never returns from.
pub fn calculate_damage(battle: &mut Battle, attacker: Side, vars: DamageVars) -> Calculated {
    let move_effect = battle.side(attacker).current_move.effect;
    let mut defense = vars.defense;
    if move_effect == effect::EXPLODE_EFFECT {
        defense = (defense >> 1).max(1);
    }
    if move_effect == effect::OHKO_EFFECT {
        one_hit_ko_effect(battle, attacker);
        return if battle.move_missed { Calculated::Skipped } else { Calculated::Damage };
    }
    if vars.power == 0 && !matches!(move_effect, effect::TWO_TO_FIVE_ATTACKS_EFFECT | effect::EFFECT_1E) {
        return Calculated::Skipped;
    }
    let doubled_level = vars.level as u32 * 2;
    let (quotient, _) = divide(doubled_level.to_be_bytes(), 5, 4);
    let mut scaled = quotient;
    scaled[3] = scaled[3].wrapping_add(2);
    let product = multiply(multiply(u32::from_be_bytes(scaled), vars.power), vars.attack);
    let (quotient, _) = divide(product.to_be_bytes(), defense, 4);
    let (quotient, _) = divide(quotient, 50, 4);
    battle.damage = add_capped(quotient, battle.damage) + MIN_NEUTRAL_DAMAGE;
    Calculated::Damage
}

/// `CalculateDamage`'s cap. The high byte of `wDamage` is added to the quotient's low byte before
/// `wDamage` itself is added, and only the second byte of the quotient is tested for size.
fn add_capped(quotient: [u8; 4], damage: u16) -> u16 {
    const CAP: u16 = MAX_NEUTRAL_DAMAGE - MIN_NEUTRAL_DAMAGE;
    let [_, q1, mut q2, q3] = quotient;
    let (q3, carry) = q3.overflowing_add((damage >> 8) as u8);
    if carry {
        q2 = q2.wrapping_add(1);
        if q2 == 0 {
            return CAP;
        }
    }
    if q1 != 0 || u16::from_be_bytes([q2, q3]) > CAP {
        return CAP;
    }
    match u16::from_be_bytes([q2, q3]).checked_add(damage) {
        Some(sum) if sum <= CAP => sum,
        _ => CAP,
    }
}

/// `OneHitKOEffect_`: 65535 damage if the user is at least as fast as the target, and otherwise a
/// miss.
pub fn one_hit_ko_effect(battle: &mut Battle, attacker: Side) {
    battle.damage = 0;
    battle.critical_hit_or_ohko = CriticalHitOrOhko::FailedOhko;
    if battle.side(attacker).mon.stats[stat::SPEED] < battle.side(attacker.other()).mon.stats[stat::SPEED] {
        battle.move_missed = true;
    } else {
        battle.damage = 0xFFFF;
        battle.critical_hit_or_ohko = CriticalHitOrOhko::SuccessfulOhko;
    }
}

const STAB_DAMAGE: u8 = 1 << 7;

/// `AdjustDamageForMoveType`: half again for STAB, then every `TypeEffects` row for the move's
/// type whose defender is either of the target's types, in the table's order, each `* n / 10`
/// in sixteen bits. `wDamageMultipliers` keeps the STAB bit and the *last* row's multiplier, not
/// the product. Damage that comes out 0 from a row is a miss.
pub fn adjust_damage_for_move_type(battle: &mut Battle, attacker: Side) {
    let user_types = battle.side(attacker).mon.types;
    let target_types = battle.side(attacker.other()).mon.types;
    let move_type = battle.side(attacker).current_move.move_type;
    if user_types.contains(&move_type) {
        battle.damage = battle.damage.wrapping_add(battle.damage >> 1);
        battle.damage_multipliers |= STAB_DAMAGE;
    }
    for (attacking, defending, multiplier) in matchups() {
        if attacking != move_type || !target_types.contains(&defending) {
            continue;
        }
        battle.damage_multipliers = multiplier + (battle.damage_multipliers & STAB_DAMAGE);
        let (quotient, _) = divide(multiply(battle.damage as u32, multiplier).to_be_bytes(), 10, 4);
        battle.damage = u16::from_be_bytes([quotient[2], quotient[3]]);
        if battle.damage == 0 {
            battle.move_missed = true;
        }
    }
}

/// `$10`: what `AIGetTypeEffectiveness` answers when no row applies, where `EFFECTIVE` is 10.
pub const AI_NEUTRAL: u8 = 0x10;
const EFFECTIVE: u16 = 10;

/// `AIGetTypeEffectiveness`: the enemy's move type against the player's mon, as the product of
/// every matching row in tenths, and `AI_NEUTRAL` where that comes to neutral.
pub fn ai_get_type_effectiveness(battle: &Battle) -> u8 {
    let move_type = battle.enemy.current_move.move_type;
    let types = battle.player.mon.types;
    let mut rows = matchups().into_iter()
        .filter(move |&(attacking, defending, _)| attacking == move_type && types.contains(&defending));
    if battle.cartridge_bugs {
        // The cartridge answers the first matching row alone, so a dual type is misread.
        return rows.next().map_or(AI_NEUTRAL, |(_, _, multiplier)| multiplier);
    }
    match rows.fold(EFFECTIVE, |product, (_, _, multiplier)| product * multiplier as u16 / EFFECTIVE) {
        EFFECTIVE => AI_NEUTRAL,
        product => product as u8,
    }
}

/// `85 percent + 1`: the least random factor `RandomizeDamage` keeps.
const MIN_DAMAGE_FACTOR: u8 = 217;

/// `RandomizeDamage`: damage of 2 or more times a random factor from 217 to 255, over 255. A
/// factor is a random byte rotated right, drawn again below 217.
pub fn randomize_damage(battle: &mut Battle, rng: &mut impl Rng) {
    if battle.damage < 2 {
        return;
    }
    let factor = loop {
        let factor = rng.random().rotate_right(1);
        if factor >= MIN_DAMAGE_FACTOR {
            break factor;
        }
    };
    let (quotient, _) = divide(multiply(battle.damage as u32, factor).to_be_bytes(), 255, 4);
    battle.damage = u16::from_be_bytes([quotient[2], quotient[3]]);
}

/// Where `PlayerCalcMoveDamage` and `EnemyCalcMoveDamage` go on to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MoveDamage {
    /// `GetPlayerAnimationType`: the move landed.
    Hit,
    /// `PlayPlayerMoveAnimation`: Explosion or Self-Destruct missed, which still animates.
    ExplosionMissed,
    /// `PlayerCheckIfFlyOrChargeEffect` with `wMoveMissed` set.
    Missed,
    /// `PlayerCheckIfFlyOrChargeEffect` for a move with no power, before any hit test.
    NoDamage,
}

/// `PlayerCalcMoveDamage` and `EnemyCalcMoveDamage` through `HandleIfPlayerMoveMissed`: Super Fang
/// and the fixed-damage moves go straight to the hit test, Counter settles itself, and every other
/// move is a critical hit roll, the damage and its type and random adjustments, then the hit test.
/// The enemy's copy swaps the two mons' levels around the calculation and back before either exit,
/// which nothing in between reads.
pub fn calc_move_damage(battle: &mut Battle, party: &[PartyMon], attacker: Side, rng: &mut impl Rng) -> MoveDamage {
    let move_effect = battle.side(attacker).current_move.effect;
    if matches!(move_effect, effect::SUPER_FANG_EFFECT | effect::SPECIAL_DAMAGE_EFFECT) {
        move_hit_test(battle, attacker, rng);
    } else {
        critical_hit_test(battle, attacker, rng);
        if handle_counter_move(battle, attacker, rng) == Counter::NotCounter {
            let Some(vars) = get_damage_vars(battle, party, attacker) else {
                return MoveDamage::NoDamage;
            };
            if calculate_damage(battle, attacker, vars) == Calculated::Skipped {
                return if battle.move_missed { MoveDamage::Missed } else { MoveDamage::NoDamage };
            }
            adjust_damage_for_move_type(battle, attacker);
            randomize_damage(battle, rng);
            let potential = battle.damage;
            move_hit_test(battle, attacker, rng);
            // Jump Kick's crash is an eighth of this, which the cartridge's miss has already zeroed.
            if battle.move_missed && move_effect == effect::JUMP_KICK_EFFECT && !battle.cartridge_bugs {
                battle.damage = potential;
            }
        }
    }
    // A trapping move that hits ends the target's recharge; the cartridge's `trapping_effect` has
    // already ended it, hit or miss.
    if !battle.cartridge_bugs && !battle.move_missed && move_effect == effect::TRAPPING_EFFECT {
        battle.side_mut(attacker.other()).status2.remove(Status2::NEEDS_TO_RECHARGE);
    }
    match (battle.move_missed, move_effect) {
        (false, _) => MoveDamage::Hit,
        (true, effect::EXPLODE_EFFECT) => MoveDamage::ExplosionMissed,
        (true, _) => MoveDamage::Missed,
    }
}

#[cfg(test)]
mod tests {
    use poke_core::moves::MoveData;
    use serde_json::{json, Value};
    use crate::rng::GameRng;
    use super::super::apply::apply_damage_to_pokemon;
    use super::super::fixture::{each_case, side};
    use super::super::turn::handle_self_confusion_damage;
    use super::super::{Arena, Status1};
    use super::*;

    const FLYING: u8 = 2;

    fn arena(player: PokemonMoveName, enemy: PokemonMoveName) -> Arena {
        let mut arena = Arena::baseline();
        arena.battle.cartridge_bugs = false;
        arena.battle.player.current_move = MoveData::of_move(player);
        arena.battle.player.selected_move = player as u8;
        arena.battle.enemy.current_move = MoveData::of_move(enemy);
        arena.battle.enemy.selected_move = enemy as u8;
        arena
    }

    #[test]
    fn focus_energy_doubles_the_critical_hit_rate() {
        // Tauros: half its base speed of 110, doubled, doubled again, then halved for Tackle.
        let mut arena = arena(PokemonMoveName::Tackle, PokemonMoveName::Tackle);
        arena.battle.player.status2 |= Status2::GETTING_PUMPED;
        critical_hit_test(&mut arena.battle, Side::Player, &mut GameRng::tape(vec![109u8.rotate_right(3)]));
        assert_eq!(arena.battle.critical_hit_or_ohko, CriticalHitOrOhko::CriticalHit);
    }

    #[test]
    fn a_doubled_defense_stops_at_999() {
        let mut arena = arena(PokemonMoveName::Tackle, PokemonMoveName::Tackle);
        arena.battle.enemy.mon.stats[stat::DEFENSE] = 600;
        arena.battle.enemy.status3 |= Status3::HAS_REFLECT_UP;
        let vars = get_damage_vars(&mut arena.battle, &arena.party, Side::Player).unwrap();
        assert_eq!(vars.defense, (999 >> 2) as u8);
    }

    #[test]
    fn a_scaled_defense_is_never_0() {
        let mut arena = arena(PokemonMoveName::Tackle, PokemonMoveName::Tackle);
        arena.battle.player.mon.stats[stat::ATTACK] = 300;
        arena.battle.enemy.mon.stats[stat::DEFENSE] = 3;
        let vars = get_damage_vars(&mut arena.battle, &arena.party, Side::Player).unwrap();
        assert_eq!(vars.defense, 1);
        calculate_damage(&mut arena.battle, Side::Player, vars);
        assert!(arena.battle.damage > 0);
    }

    #[test]
    fn a_critical_hit_reads_the_transformed_stats() {
        let mut arena = arena(PokemonMoveName::Tackle, PokemonMoveName::Tackle);
        arena.battle.player.status3 |= Status3::TRANSFORMED;
        arena.battle.player.unmodified_stats[stat::ATTACK] = 200;
        arena.battle.enemy.unmodified_stats[stat::DEFENSE] = 77;
        arena.battle.critical_hit_or_ohko = CriticalHitOrOhko::CriticalHit;
        let vars = get_damage_vars(&mut arena.battle, &arena.party, Side::Player).unwrap();
        assert_eq!((vars.attack, vars.defense), (200, 77));
    }

    #[test]
    fn the_ai_reads_both_types() {
        let mut arena = arena(PokemonMoveName::Tackle, PokemonMoveName::Submission);
        arena.battle.player.mon.types = [NORMAL, FLYING];
        assert_eq!(ai_get_type_effectiveness(&arena.battle), AI_NEUTRAL);
        arena.battle.player.mon.types = [NORMAL, NORMAL];
        assert_eq!(ai_get_type_effectiveness(&arena.battle), 20);
    }

    /// Counter against what `enemy`'s move just did to the player, after `setup`.
    fn counter(enemy: PokemonMoveName, setup: impl FnOnce(&mut Arena)) -> Arena {
        let mut arena = arena(PokemonMoveName::Counter, enemy);
        setup(&mut arena);
        handle_counter_move(&mut arena.battle, Side::Player, &mut GameRng::tape(vec![]));
        arena
    }

    #[test]
    fn counter_doubles_what_the_target_did_to_the_user() {
        let arena = counter(PokemonMoveName::Tackle, |arena| {
            arena.battle.damage = 30;
            apply_damage_to_pokemon(&mut arena.battle, Side::Player, Side::Enemy);
        });
        assert_eq!((arena.battle.move_missed, arena.battle.damage), (false, 60));
    }

    #[test]
    fn counter_misses_what_the_user_did_itself() {
        let arena = counter(PokemonMoveName::Tackle, |arena| {
            arena.battle.player.current_move = MoveData::of_move(PokemonMoveName::Tackle);
            arena.battle.damage = 30;
            apply_damage_to_pokemon(&mut arena.battle, Side::Enemy, Side::Player);
            arena.battle.player.current_move = MoveData::of_move(PokemonMoveName::Counter);
        });
        assert!(arena.battle.move_missed);
    }

    #[test]
    fn counter_misses_a_confused_target_that_hurt_itself() {
        let arena = counter(PokemonMoveName::Thunderbolt, |arena| {
            arena.battle.enemy.status1 |= Status1::CONFUSED;
            let party = arena.party.clone();
            handle_self_confusion_damage(&mut arena.battle, &party, Side::Enemy);
            assert!(arena.battle.damage > 0);
        });
        assert!(arena.battle.move_missed);
        assert_eq!(arena.battle.enemy.current_move, MoveData::of_move(PokemonMoveName::Thunderbolt));
    }

    /// The damage a confused player does itself with Reflect up on `reflecting` sides.
    fn confusion_damage(reflecting: &[Side]) -> u16 {
        let mut arena = arena(PokemonMoveName::Tackle, PokemonMoveName::Tackle);
        for &side in reflecting {
            arena.battle.side_mut(side).status3 |= Status3::HAS_REFLECT_UP;
        }
        let party = arena.party.clone();
        handle_self_confusion_damage(&mut arena.battle, &party, Side::Player);
        let player = &arena.battle.player.mon;
        player.stats[stat::MAX_HP] - player.hp
    }

    #[test]
    fn confusion_damage_is_under_the_users_own_reflect() {
        let plain = confusion_damage(&[]);
        assert!(confusion_damage(&[Side::Player]) < plain);
        assert_eq!(confusion_damage(&[Side::Enemy]), plain);
    }

    #[test]
    fn every_harvested_case_of_critical_hit_test() {
        each_case(include_str!("../../../fixtures/battle/critical_hit_test.jsonl"), |arena, input, rng| {
            critical_hit_test(&mut arena.battle, side(input), rng);
            Value::Null
        });
    }

    #[test]
    fn every_harvested_case_of_get_damage_vars() {
        each_case(include_str!("../../../fixtures/battle/get_damage_vars.jsonl"), |arena, input, _| {
            json!(get_damage_vars(&mut arena.battle, &arena.party, side(input)))
        });
    }

    #[test]
    fn every_harvested_case_of_calculate_damage() {
        each_case(include_str!("../../../fixtures/battle/calculate_damage.jsonl"), |arena, input, _| {
            let vars = serde_json::from_value(input["vars"].clone()).unwrap();
            json!(calculate_damage(&mut arena.battle, side(input), vars))
        });
    }

    #[test]
    fn every_harvested_case_of_handle_counter_move() {
        each_case(include_str!("../../../fixtures/battle/handle_counter_move.jsonl"), |arena, input, rng| {
            json!(handle_counter_move(&mut arena.battle, side(input), rng))
        });
    }

    #[test]
    fn every_harvested_case_of_adjust_damage_for_move_type() {
        each_case(include_str!("../../../fixtures/battle/adjust_damage_for_move_type.jsonl"), |arena, input, _| {
            adjust_damage_for_move_type(&mut arena.battle, side(input));
            Value::Null
        });
    }

    #[test]
    fn every_harvested_case_of_ai_get_type_effectiveness() {
        each_case(include_str!("../../../fixtures/battle/ai_get_type_effectiveness.jsonl"), |arena, _, _| {
            json!(ai_get_type_effectiveness(&arena.battle))
        });
    }

    #[test]
    fn every_harvested_case_of_randomize_damage() {
        each_case(include_str!("../../../fixtures/battle/randomize_damage.jsonl"), |arena, _, rng| {
            randomize_damage(&mut arena.battle, rng);
            Value::Null
        });
    }

    #[test]
    fn every_harvested_case_of_calc_move_damage() {
        each_case(include_str!("../../../fixtures/battle/calc_move_damage.jsonl"), |arena, input, rng| {
            let exit = calc_move_damage(&mut arena.battle, &arena.party, side(input), rng);
            json!(match exit {
                MoveDamage::Hit => "Hit",
                MoveDamage::ExplosionMissed => "ExplosionMissed",
                MoveDamage::Missed | MoveDamage::NoDamage => "NotAnimated",
            })
        });
    }
}
