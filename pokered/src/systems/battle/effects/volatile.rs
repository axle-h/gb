//! The effects that set what lasts while a mon is out: the multi-turn moves, the screens, Mist,
//! Focus Energy, Haze, Conversion, Transform, Mimic, Disable, Pay Day, and running away.

use poke_core::move_name::PokemonMoveName;
use crate::party::PartyMon;
use crate::rng::Rng;
use crate::systems::math::{add_bcd, divide};
use super::super::accuracy::move_hit_test;
use super::super::ai::CANNOT_MOVE;
use super::super::modified_stats::apply_penalty_and_badge_boost_to;
use super::super::{effect, stat, status, BattleKind, Battle, Side, Status1, Status2, Status3, BASE_STAT_LEVEL, PP_MASK};
use super::{clear_hyper_beam, conditional_but_it_failed, read_player_mon_cur_hp_and_status, BattleText, MimicMenu};

/// `BideEffect`: 2 or 3 turns of storing energy from nothing, and *both* sides' move effects zeroed.
pub fn bide_effect(battle: &mut Battle, user: Side, rng: &mut impl Rng) -> Vec<BattleText> {
    let side = battle.side_mut(user);
    side.status1 |= Status1::STORING_ENERGY;
    side.bide_accumulated_damage = 0;
    battle.player.current_move.effect = 0;
    battle.enemy.current_move.effect = 0;
    battle.side_mut(user).num_attacks_left = (rng.random() & 1) + 2;
    vec![]
}

/// `ThrashPetalDanceEffect`: 2 or 3 more turns.
pub fn thrash_petal_dance_effect(battle: &mut Battle, user: Side, rng: &mut impl Rng) -> Vec<BattleText> {
    let side = battle.side_mut(user);
    side.status1 |= Status1::THRASHING_ABOUT;
    side.num_attacks_left = (rng.random() & 1) + 2;
    vec![]
}

/// `SwitchAndTeleportEffect`. Only in a wild battle: a user at least the other's level always gets
/// away, and otherwise does when a random byte drawn below the sum of the levels plus one, in a
/// byte, is at least a quarter of the other's level. Getting away copies the player's HP and status
/// back to its party and sets `wEscapedFromBattle`. The enemy's level is `wCurEnemyLevel`, which is
/// the enemy mon's.
pub fn switch_and_teleport_effect(battle: &mut Battle, party: &mut [PartyMon], user: Side, rng: &mut impl Rng)
                                  -> Vec<BattleText> {
    let move_num = battle.side(user).current_move.animation;
    let teleport = move_num == PokemonMoveName::Teleport as u8;
    if battle.kind != BattleKind::Wild {
        return if !teleport {
            vec![BattleText::IsUnaffectedText]
        } else if user == Side::Player {
            vec![BattleText::ButItFailedText]
        } else {
            conditional_but_it_failed(battle)
        };
    }
    let (level, other) = (battle.side(user).mon.level, battle.side(user.other()).mon.level);
    if level < other {
        let bound = level.wrapping_add(other).wrapping_add(1);
        let random = loop {
            let random = rng.random();
            if random < bound {
                break random;
            }
        };
        if random < other >> 2 {
            return vec![if teleport { BattleText::ButItFailedText } else { BattleText::DidntAffectText }];
        }
    }
    read_player_mon_cur_hp_and_status(battle, party);
    battle.escaped_from_battle = true;
    vec![match move_num {
        _ if teleport => BattleText::RanFromBattleText,
        _ if move_num == PokemonMoveName::Roar as u8 => BattleText::RanAwayScaredText,
        _ => BattleText::WasBlownAwayText,
    }]
}

/// `TwoToFiveAttacksEffect`, once per move: 2 hits for Double Kick and Twineedle, whose effect
/// becomes the poison side effect for its second half, and otherwise a random byte's low two bits,
/// drawn once more if they are 2 or 3, plus 2.
pub fn two_to_five_attacks_effect(battle: &mut Battle, user: Side, rng: &mut impl Rng) -> Vec<BattleText> {
    let side = battle.side_mut(user);
    if side.status1.contains(Status1::ATTACKING_MULTIPLE_TIMES) {
        return vec![];
    }
    side.status1 |= Status1::ATTACKING_MULTIPLE_TIMES;
    let hits = match side.current_move.effect {
        effect::TWINEEDLE_EFFECT => {
            side.current_move.effect = effect::POISON_SIDE_EFFECT1;
            2
        }
        effect::ATTACK_TWICE_EFFECT => 2,
        _ => {
            let low = rng.random() & 3;
            (if low < 2 { low } else { rng.random() & 3 }) + 2
        }
    };
    let side = battle.side_mut(user);
    side.num_attacks_left = hits;
    side.set_num_hits(hits);
    vec![]
}

/// `ChargeEffect`: charging, and out of reach for Fly's effect or Dig by move number.
pub fn charge_effect(battle: &mut Battle, user: Side) -> Vec<BattleText> {
    let side = battle.side_mut(user);
    side.status1 |= Status1::CHARGING_UP;
    if side.current_move.effect == effect::FLY_EFFECT || side.current_move.animation == PokemonMoveName::Dig as u8 {
        side.status1 |= Status1::INVULNERABLE;
    }
    vec![BattleText::ChargeMoveEffectText]
}

/// `TrappingEffect`, once per move, before the hit test: 1 to 4 more turns follow, weighted as the
/// multi-hit moves are. The target's recharge ends when the move hits, in `calc_move_damage`.
pub fn trapping_effect(battle: &mut Battle, user: Side, rng: &mut impl Rng) -> Vec<BattleText> {
    if battle.side(user).status1.contains(Status1::USING_TRAPPING_MOVE) {
        return vec![];
    }
    // The cartridge ends it here, so a miss frees the target too.
    if battle.ruleset.is_gen1() {
        clear_hyper_beam(battle, user);
    }
    let side = battle.side_mut(user);
    side.status1 |= Status1::USING_TRAPPING_MOVE;
    let low = rng.random() & 3;
    side.num_attacks_left = (if low < 2 { low } else { rng.random() & 3 }) + 1;
    vec![]
}

/// `MistEffect_`.
pub fn mist_effect(battle: &mut Battle, user: Side) -> Vec<BattleText> {
    let side = battle.side_mut(user);
    if side.status2.contains(Status2::PROTECTED_BY_MIST) {
        return vec![BattleText::ButItFailedText];
    }
    side.status2 |= Status2::PROTECTED_BY_MIST;
    vec![BattleText::ShroudedInMistText]
}

/// `FocusEnergyEffect_`.
pub fn focus_energy_effect(battle: &mut Battle, user: Side) -> Vec<BattleText> {
    let side = battle.side_mut(user);
    if side.status2.contains(Status2::GETTING_PUMPED) {
        return vec![BattleText::ButItFailedText];
    }
    side.status2 |= Status2::GETTING_PUMPED;
    vec![BattleText::GettingPumpedText]
}

/// `ReflectLightScreenEffect_`.
pub fn reflect_light_screen_effect(battle: &mut Battle, user: Side) -> Vec<BattleText> {
    let side = battle.side_mut(user);
    let (screen, text) = if side.current_move.effect == effect::LIGHT_SCREEN_EFFECT {
        (Status3::HAS_LIGHT_SCREEN_UP, BattleText::LightScreenProtectedText)
    } else {
        (Status3::HAS_REFLECT_UP, BattleText::ReflectGainedArmorText)
    };
    if side.status3.contains(screen) {
        return vec![BattleText::ButItFailedText];
    }
    side.status3 |= screen;
    vec![text]
}

/// `HazeEffect_`: both sides' stat modifiers and stats back to unmodified, the target's status
/// cured, both disabled moves forgotten, and both sides' confusion, X Accuracy, Mist, Focus Energy,
/// Leech Seed, Toxic and screens ended. The user's own status and the toxic counter stay. A target
/// it wakes or thaws loses its turn, which the asm says is intended.
pub fn haze_effect(battle: &mut Battle, user: Side, badges: u8) -> Vec<BattleText> {
    for side in [&mut battle.player, &mut battle.enemy] {
        side.stat_mods = [BASE_STAT_LEVEL; 6];
        side.mon.stats[stat::ATTACK..].copy_from_slice(&side.unmodified_stats[stat::ATTACK..]);
    }
    let target = battle.side_mut(user.other());
    if target.mon.status & (status::FRZ | status::SLP_MASK) != 0 {
        target.selected_move = CANNOT_MOVE;
    }
    target.mon.status = 0;
    for side in [&mut battle.player, &mut battle.enemy] {
        side.disabled_move = 0;
        side.disabled_move_number = 0;
        side.status1.remove(Status1::CONFUSED);
        side.status2.remove(Status2::USING_X_ACCURACY | Status2::PROTECTED_BY_MIST | Status2::GETTING_PUMPED | Status2::SEEDED);
        side.status3.remove(Status3::BADLY_POISONED | Status3::HAS_LIGHT_SCREEN_UP | Status3::HAS_REFLECT_UP);
    }
    // The cartridge drops the player's badge boosts, and the penalty of the status the user keeps.
    if !battle.ruleset.is_gen1() {
        for side in [Side::Player, Side::Enemy] {
            for which in 0..4 {
                apply_penalty_and_badge_boost_to(battle, side, which, badges);
            }
        }
    }
    vec![BattleText::StatusChangesEliminatedText]
}

/// `ConversionEffect_`: the target's types, unless the target is out of reach, which the enemy's
/// copy reads from the player and the player's from the enemy.
pub fn conversion_effect(battle: &mut Battle, user: Side) -> Vec<BattleText> {
    let target = battle.side(user.other());
    if target.status1.contains(Status1::INVULNERABLE) {
        return vec![BattleText::ButItFailedText];
    }
    let types = target.mon.types;
    battle.side_mut(user).mon.types = types;
    vec![BattleText::ConvertedTypeText]
}

/// `TransformEffect_`. A target out of reach makes it fail, and the player's turn zeroes
/// `wPlayerMoveListIndex` on the way. The user takes the target's species, types, catch rate, moves, DVs, and stats but for level and HP, 5 PP
/// for each move up to the first empty slot, and the target's unmodified stats and stat modifiers;
/// the enemy keeps its own DVs aside.
pub fn transform_effect(battle: &mut Battle, user: Side) -> Vec<BattleText> {
    if user == Side::Player {
        battle.player.move_list_index = 0;
    }
    // The cartridge tests the player's own flags on the player's turn and nothing on the enemy's.
    let out_of_reach = match battle.ruleset.is_gen1() {
        true => user == Side::Player && battle.player.status1.contains(Status1::INVULNERABLE),
        false => battle.side(user.other()).status1.contains(Status1::INVULNERABLE),
    };
    if out_of_reach {
        return vec![BattleText::ButItFailedText];
    }
    let target = battle.side(user.other()).clone();
    if user == Side::Enemy {
        battle.transformed_enemy_original_dvs = battle.enemy.mon.dvs;
    }
    let side = battle.side_mut(user);
    side.status3 |= Status3::TRANSFORMED;
    side.mon.species = target.mon.species;
    side.mon.types = target.mon.types;
    side.mon.catch_rate = target.mon.catch_rate;
    side.mon.moves = target.mon.moves;
    side.mon.dvs = target.mon.dvs;
    side.mon.stats[stat::ATTACK..].copy_from_slice(&target.mon.stats[stat::ATTACK..]);
    let known = target.mon.moves.iter().take_while(|name| name.is_some()).count();
    side.mon.pp = std::array::from_fn(|slot| if slot < known { 5 } else { 0 });
    side.unmodified_stats[stat::ATTACK..].copy_from_slice(&target.unmodified_stats[stat::ATTACK..]);
    side.stat_mods = target.stat_mods;
    vec![BattleText::TransformedText]
}

/// `MimicEffect`: a hit test, then out of reach fails it. The enemy copies a random one of the
/// player's moves into the slot it used; the player chooses one of the enemy's from the move menu,
/// into the slot the fight menu's cursor was on. Its PP is not touched.
pub fn mimic_effect(battle: &mut Battle, user: Side, menu: MimicMenu, rng: &mut impl Rng) -> Vec<BattleText> {
    if !mimic_lands(battle, user, rng) {
        return vec![BattleText::ButItFailedText];
    }
    match user {
        Side::Enemy => {
            let copied = loop {
                if let Some(name) = battle.player.mon.moves[(rng.random() & 3) as usize] {
                    break Some(name);
                }
            };
            let slot = battle.enemy.move_list_index;
            battle.enemy.mon.moves[slot as usize] = copied;
        }
        Side::Player => player_mimic_copy(battle, menu),
    }
    vec![BattleText::MimicLearnedMoveText]
}

/// `MimicEffect` as far as `.letPlayerChooseMove`: whether the move lands, which is all that is
/// drawn before the player's menu.
pub fn mimic_lands(battle: &mut Battle, user: Side, rng: &mut impl Rng) -> bool {
    move_hit_test(battle, user, rng);
    !battle.move_missed && !battle.side(user.other()).status1.contains(Status1::INVULNERABLE)
}

/// `MimicEffect` after the player's menu: the enemy's move `chosen` into the player's slot `cursor`.
pub fn player_mimic_copy(battle: &mut Battle, menu: MimicMenu) {
    battle.player.mon.moves[menu.cursor as usize] = battle.enemy.mon.moves[menu.chosen as usize];
}

/// `DisableEffect`: a hit test, then nothing if the target has a move disabled already. A random
/// slot with a move is drawn; when the enemy disables, it is drawn again for a move with no PP
/// byte at all, and fails if every move is out of PP. 1 to 8 turns.
pub fn disable_effect(battle: &mut Battle, user: Side, rng: &mut impl Rng) -> Vec<BattleText> {
    move_hit_test(battle, user, rng);
    if battle.move_missed || battle.side(user.other()).disabled_move != 0 {
        return vec![BattleText::ButItFailedText];
    }
    let target = battle.side_mut(user.other());
    let (slot, name) = loop {
        let slot = rng.random() & 3;
        let Some(name) = target.mon.moves[slot as usize] else { continue };
        if user == Side::Enemy {
            if target.mon.pp.iter().fold(0, |any, &pp| any | pp) & PP_MASK == 0 {
                return vec![BattleText::ButItFailedText];
            }
            if target.mon.pp[slot as usize] == 0 {
                continue;
            }
        }
        break (slot, name);
    };
    let turns = (rng.random() & 7) + 1;
    target.disabled_move = (slot + 1) << 4 | turns;
    target.disabled_move_number = name as u8;
    vec![BattleText::MoveWasDisabledText]
}

/// `PayDayEffect_`: twice the user's level, in a byte, as three BCD bytes, onto the battle's total.
pub fn pay_day_effect(battle: &mut Battle, user: Side) -> Vec<BattleText> {
    let doubled = battle.side(user).mon.level.wrapping_mul(2);
    let (hundreds, rest) = divide([0, 0, 0, doubled], 100, 4);
    let (tens, ones) = divide([0, 0, 0, rest], 10, 4);
    let money = [0, hundreds[3], tens[3] << 4 | ones];
    add_bcd(&mut battle.total_pay_day_money, &money);
    vec![BattleText::CoinsScatteredText]
}

#[cfg(test)]
mod tests {
    use crate::world::Ruleset;
    use crate::rng::GameRng;
    use super::super::super::damage::calc_move_damage;
    use super::super::tests::using;
    use super::*;

    #[test]
    fn transform_fails_on_a_target_out_of_reach_and_only_then() {
        for user in [Side::Player, Side::Enemy] {
            let mut arena = using(user, PokemonMoveName::Transform);
            arena.battle.side_mut(user.other()).status1 |= Status1::INVULNERABLE;
            assert_eq!(transform_effect(&mut arena.battle, user), vec![BattleText::ButItFailedText], "{user:?}");
            assert!(!arena.battle.side(user).status3.contains(Status3::TRANSFORMED));

            arena.battle.side_mut(user.other()).status1.remove(Status1::INVULNERABLE);
            arena.battle.side_mut(user).status1 |= Status1::INVULNERABLE;
            assert_eq!(transform_effect(&mut arena.battle, user), vec![BattleText::TransformedText], "{user:?}");
        }
    }

    /// The player's Wrap on an enemy that must recharge, `hit_roll` its hit test's random byte:
    /// whether the enemy still must.
    fn wrap_leaves_it_recharging(hit_roll: u8) -> bool {
        let mut arena = using(Side::Player, PokemonMoveName::Wrap);
        arena.battle.enemy.status2 |= Status2::NEEDS_TO_RECHARGE;
        trapping_effect(&mut arena.battle, Side::Player, &mut GameRng::tape(vec![0]));
        let party = arena.party.clone();
        calc_move_damage(&mut arena.battle, &party, Side::Player, &mut GameRng::tape(vec![0xFF, 0xFF, hit_roll]));
        assert_eq!(arena.battle.move_missed, hit_roll == 0xFF);
        arena.battle.enemy.status2.contains(Status2::NEEDS_TO_RECHARGE)
    }

    #[test]
    fn haze_takes_the_turn_from_a_target_it_wakes() {
        for (user, frozen) in [(Side::Player, false), (Side::Enemy, true)] {
            let mut arena = using(user, PokemonMoveName::Haze);
            let target = arena.battle.side_mut(user.other());
            target.mon.status = if frozen { status::FRZ } else { 3 };
            target.selected_move = 0;
            haze_effect(&mut arena.battle, user, 0);
            let target = arena.battle.side(user.other());
            assert_eq!((target.mon.status, target.selected_move), (0, CANNOT_MOVE), "{user:?}");
        }
    }

    #[test]
    fn haze_keeps_the_badge_boosts_and_the_users_own_penalty() {
        let hazed = |ruleset| {
            let mut arena = using(Side::Player, PokemonMoveName::Haze);
            arena.battle.ruleset = ruleset;
            arena.battle.player.mon.status = status::PAR;
            arena.battle.enemy.mon.status = status::BRN;
            haze_effect(&mut arena.battle, Side::Player, 0xFF);
            arena.battle
        };
        let battle = hazed(Ruleset::Modern);
        let (unmodified, stats) = (battle.player.unmodified_stats, battle.player.mon.stats);
        let boosted = |value: u16| value + (value >> 3);
        assert_eq!(stats[stat::ATTACK], boosted(unmodified[stat::ATTACK]));
        assert_eq!(stats[stat::SPEED], boosted(unmodified[stat::SPEED] >> 2));
        assert_eq!(battle.enemy.mon.stats, battle.enemy.unmodified_stats);
        let battle = hazed(Ruleset::Gen1);
        assert_eq!(battle.player.mon.stats, battle.player.unmodified_stats);
    }

    #[test]
    fn a_missed_wrap_leaves_the_target_recharging() {
        assert!(wrap_leaves_it_recharging(0xFF));
        assert!(!wrap_leaves_it_recharging(0));
    }
}
