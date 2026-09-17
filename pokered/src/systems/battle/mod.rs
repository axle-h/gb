//! The arithmetic of a battle: `engine/battle/core.asm`'s damage, accuracy, stat and turn-order
//! routines, `experience.asm`, `read_trainer_party.asm`, `trainer_ai.asm` and the move effects,
//! over a battle state shaped like the battle rather than like WRAM.
//!
//! `hWhoseTurn` is a `Side` parameter: a routine that reads `wPlayerX` on the player's turn and
//! `wEnemyX` on the enemy's takes the attacker and reads `battle.side(attacker)`. What is exact is
//! every number and every random byte drawn, in order; what is not here is anything drawn, printed
//! or animated, which a routine hands back as a value for the battle mode to show.

pub mod accuracy;
pub mod ai;
pub mod apply;
pub mod damage;
pub mod effects;
pub mod enemy;
pub mod escape;
pub mod experience;
pub mod modified_stats;
pub mod safari;
pub mod trainer;
pub mod turn;
pub mod turn_order;

use bitflags::bitflags;
use poke_core::move_name::PokemonMoveName;
use poke_core::moves::MoveData;
use poke_core::species::PokemonSpecies;
use serde::{Deserialize, Serialize};
use crate::party::{PartyMon, NUM_MOVES, NUM_STATS};
use crate::systems::stats::Dvs;

/// `hWhoseTurn`: 0 for the player, 1 for the enemy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Side {
    Player,
    Enemy,
}

impl Side {
    pub fn other(self) -> Side {
        match self {
            Side::Player => Side::Enemy,
            Side::Enemy => Side::Player,
        }
    }
}

/// `SLP_MASK` and the other non-volatile status bits of a mon's status byte.
pub mod status {
    pub const SLP_MASK: u8 = 0b111;
    pub const PSN: u8 = 1 << 3;
    pub const BRN: u8 = 1 << 4;
    pub const FRZ: u8 = 1 << 5;
    pub const PAR: u8 = 1 << 6;
}

/// `constants/move_effect_constants.asm`: a move's `MOVE_EFFECT` byte.
pub mod effect {
    pub const NO_ADDITIONAL_EFFECT: u8 = 0x00;
    pub const EFFECT_01: u8 = 0x01;
    pub const POISON_SIDE_EFFECT1: u8 = 0x02;
    pub const DRAIN_HP_EFFECT: u8 = 0x03;
    pub const BURN_SIDE_EFFECT1: u8 = 0x04;
    pub const FREEZE_SIDE_EFFECT1: u8 = 0x05;
    pub const PARALYZE_SIDE_EFFECT1: u8 = 0x06;
    pub const EXPLODE_EFFECT: u8 = 0x07;
    pub const DREAM_EATER_EFFECT: u8 = 0x08;
    pub const MIRROR_MOVE_EFFECT: u8 = 0x09;
    pub const ATTACK_UP1_EFFECT: u8 = 0x0A;
    pub const DEFENSE_UP1_EFFECT: u8 = 0x0B;
    pub const SPEED_UP1_EFFECT: u8 = 0x0C;
    pub const SPECIAL_UP1_EFFECT: u8 = 0x0D;
    pub const ACCURACY_UP1_EFFECT: u8 = 0x0E;
    pub const EVASION_UP1_EFFECT: u8 = 0x0F;
    pub const PAY_DAY_EFFECT: u8 = 0x10;
    pub const SWIFT_EFFECT: u8 = 0x11;
    pub const ATTACK_DOWN1_EFFECT: u8 = 0x12;
    pub const DEFENSE_DOWN1_EFFECT: u8 = 0x13;
    pub const SPEED_DOWN1_EFFECT: u8 = 0x14;
    pub const SPECIAL_DOWN1_EFFECT: u8 = 0x15;
    pub const ACCURACY_DOWN1_EFFECT: u8 = 0x16;
    pub const EVASION_DOWN1_EFFECT: u8 = 0x17;
    pub const CONVERSION_EFFECT: u8 = 0x18;
    pub const HAZE_EFFECT: u8 = 0x19;
    pub const BIDE_EFFECT: u8 = 0x1A;
    pub const THRASH_PETAL_DANCE_EFFECT: u8 = 0x1B;
    pub const SWITCH_AND_TELEPORT_EFFECT: u8 = 0x1C;
    pub const TWO_TO_FIVE_ATTACKS_EFFECT: u8 = 0x1D;
    pub const EFFECT_1E: u8 = 0x1E;
    pub const FLINCH_SIDE_EFFECT1: u8 = 0x1F;
    pub const SLEEP_EFFECT: u8 = 0x20;
    pub const POISON_SIDE_EFFECT2: u8 = 0x21;
    pub const BURN_SIDE_EFFECT2: u8 = 0x22;
    pub const FREEZE_SIDE_EFFECT2: u8 = 0x23;
    pub const PARALYZE_SIDE_EFFECT2: u8 = 0x24;
    pub const FLINCH_SIDE_EFFECT2: u8 = 0x25;
    pub const OHKO_EFFECT: u8 = 0x26;
    pub const CHARGE_EFFECT: u8 = 0x27;
    pub const SUPER_FANG_EFFECT: u8 = 0x28;
    pub const SPECIAL_DAMAGE_EFFECT: u8 = 0x29;
    pub const TRAPPING_EFFECT: u8 = 0x2A;
    pub const FLY_EFFECT: u8 = 0x2B;
    pub const ATTACK_TWICE_EFFECT: u8 = 0x2C;
    pub const JUMP_KICK_EFFECT: u8 = 0x2D;
    pub const MIST_EFFECT: u8 = 0x2E;
    pub const FOCUS_ENERGY_EFFECT: u8 = 0x2F;
    pub const RECOIL_EFFECT: u8 = 0x30;
    pub const CONFUSION_EFFECT: u8 = 0x31;
    pub const ATTACK_UP2_EFFECT: u8 = 0x32;
    pub const DEFENSE_UP2_EFFECT: u8 = 0x33;
    pub const SPEED_UP2_EFFECT: u8 = 0x34;
    pub const SPECIAL_UP2_EFFECT: u8 = 0x35;
    pub const ACCURACY_UP2_EFFECT: u8 = 0x36;
    pub const EVASION_UP2_EFFECT: u8 = 0x37;
    pub const HEAL_EFFECT: u8 = 0x38;
    pub const TRANSFORM_EFFECT: u8 = 0x39;
    pub const ATTACK_DOWN2_EFFECT: u8 = 0x3A;
    pub const DEFENSE_DOWN2_EFFECT: u8 = 0x3B;
    pub const SPEED_DOWN2_EFFECT: u8 = 0x3C;
    pub const SPECIAL_DOWN2_EFFECT: u8 = 0x3D;
    pub const ACCURACY_DOWN2_EFFECT: u8 = 0x3E;
    pub const EVASION_DOWN2_EFFECT: u8 = 0x3F;
    pub const LIGHT_SCREEN_EFFECT: u8 = 0x40;
    pub const REFLECT_EFFECT: u8 = 0x41;
    pub const POISON_EFFECT: u8 = 0x42;
    pub const PARALYZE_EFFECT: u8 = 0x43;
    pub const ATTACK_DOWN_SIDE_EFFECT: u8 = 0x44;
    pub const DEFENSE_DOWN_SIDE_EFFECT: u8 = 0x45;
    pub const SPEED_DOWN_SIDE_EFFECT: u8 = 0x46;
    pub const SPECIAL_DOWN_SIDE_EFFECT: u8 = 0x47;
    pub const CONFUSION_SIDE_EFFECT: u8 = 0x4C;
    pub const TWINEEDLE_EFFECT: u8 = 0x4D;
    pub const SUBSTITUTE_EFFECT: u8 = 0x4F;
    pub const HYPER_BEAM_EFFECT: u8 = 0x50;
    pub const RAGE_EFFECT: u8 = 0x51;
    pub const MIMIC_EFFECT: u8 = 0x52;
    pub const METRONOME_EFFECT: u8 = 0x53;
    pub const LEECH_SEED_EFFECT: u8 = 0x54;
    pub const SPLASH_EFFECT: u8 = 0x55;
    pub const DISABLE_EFFECT: u8 = 0x56;
}

/// `SPECIAL`: a move type from here up is special.
pub const SPECIAL: u8 = 20;
pub const MIN_NEUTRAL_DAMAGE: u16 = 2;
pub const MAX_NEUTRAL_DAMAGE: u16 = 999;
/// `BASE_STAT_LEVEL`: the stat modifier that is no change.
pub const BASE_STAT_LEVEL: u8 = 7;
pub const MAX_STAT_LEVEL: u8 = 13;
/// `PP_MASK`: the PP left, below the PP Ups.
pub const PP_MASK: u8 = 0x3F;

bitflags! {
    /// `wPlayerBattleStatus1`, `wEnemyBattleStatus1`.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
    pub struct Status1: u8 {
        const STORING_ENERGY = 1 << 0;
        const THRASHING_ABOUT = 1 << 1;
        const ATTACKING_MULTIPLE_TIMES = 1 << 2;
        const FLINCHED = 1 << 3;
        const CHARGING_UP = 1 << 4;
        const USING_TRAPPING_MOVE = 1 << 5;
        const INVULNERABLE = 1 << 6;
        const CONFUSED = 1 << 7;
    }

    /// `wPlayerBattleStatus2`, `wEnemyBattleStatus2`. Bit 3 is unused and kept as found.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
    pub struct Status2: u8 {
        const USING_X_ACCURACY = 1 << 0;
        const PROTECTED_BY_MIST = 1 << 1;
        const GETTING_PUMPED = 1 << 2;
        const HAS_SUBSTITUTE_UP = 1 << 4;
        const NEEDS_TO_RECHARGE = 1 << 5;
        const USING_RAGE = 1 << 6;
        const SEEDED = 1 << 7;
        const _ = !0;
    }

    /// `wPlayerBattleStatus3`, `wEnemyBattleStatus3`. The top four bits are unused and kept.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
    pub struct Status3: u8 {
        const BADLY_POISONED = 1 << 0;
        const HAS_LIGHT_SCREEN_UP = 1 << 1;
        const HAS_REFLECT_UP = 1 << 2;
        const TRANSFORMED = 1 << 3;
        const _ = !0;
    }
}

/// `battle_struct`: the mon in battle, `wBattleMon` or `wEnemyMon`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BattleMon {
    pub species: PokemonSpecies,
    pub hp: u16,
    /// `PartyPos`, the byte a box mon keeps its level in.
    pub party_pos: u8,
    pub status: u8,
    pub types: [u8; 2],
    pub catch_rate: u8,
    pub moves: [Option<PokemonMoveName>; NUM_MOVES],
    pub dvs: Dvs,
    pub level: u8,
    /// Max HP, attack, defense, speed, special.
    pub stats: [u16; NUM_STATS],
    /// PP left in the low six bits, PP Ups in the top two.
    pub pp: [u8; NUM_MOVES],
}

/// Indices into `BattleMon::stats` and the unmodified stats beside them.
pub mod stat {
    pub const MAX_HP: usize = 0;
    pub const ATTACK: usize = 1;
    pub const DEFENSE: usize = 2;
    pub const SPEED: usize = 3;
    pub const SPECIAL: usize = 4;
}

/// `wPlayerMonStatMods` order: `MOD_ATTACK` to `MOD_EVASION`.
pub mod stat_mod {
    pub const ATTACK: usize = 0;
    pub const DEFENSE: usize = 1;
    pub const SPEED: usize = 2;
    pub const SPECIAL: usize = 3;
    pub const ACCURACY: usize = 4;
    pub const EVASION: usize = 5;
}

/// One side of a battle: its mon and everything the battle keeps about it while it is out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Combatant {
    pub mon: BattleMon,
    /// `wPlayerMonUnmodifiedLevel`: the level and stats before stat modifiers.
    pub unmodified_level: u8,
    pub unmodified_stats: [u16; NUM_STATS],
    /// `wPlayerMonStatMods`: 1 to 13, `BASE_STAT_LEVEL` for none.
    pub stat_mods: [u8; 6],
    pub status1: Status1,
    pub status2: Status2,
    pub status3: Status3,
    pub num_attacks_left: u8,
    pub confused_counter: u8,
    pub toxic_counter: u8,
    /// `wPlayerDisabledMove`: the slot from 1 in the high nybble, the turns left in the low.
    pub disabled_move: u8,
    /// `wPlayerBideAccumulatedDamage`, whose high byte is also `wPlayerNumHits`.
    pub bide_accumulated_damage: u16,
    pub substitute_hp: u8,
    /// `wPlayerSelectedMove`: a move id, or `CANNOT_MOVE`.
    pub selected_move: u8,
    pub move_list_index: u8,
    pub disabled_move_number: u8,
    pub used_move: u8,
    pub minimized: u8,
    /// `wPlayerMoveNum` to `wPlayerMoveMaxPP`: the move being used, as the routines have left it.
    pub current_move: MoveData,
}

impl Combatant {
    pub fn num_hits(&self) -> u8 {
        (self.bide_accumulated_damage >> 8) as u8
    }

    pub fn set_num_hits(&mut self, hits: u8) {
        self.bide_accumulated_damage = self.bide_accumulated_damage & 0xFF | (hits as u16) << 8;
    }
}

/// `wCriticalHitOrOHKO`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CriticalHitOrOhko {
    #[default]
    Normal,
    CriticalHit,
    SuccessfulOhko,
    FailedOhko,
}

/// `wIsInBattle`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BattleKind {
    Lost,
    Wild,
    Trainer,
}

/// `wEnemyMonBaseStats`, `wEnemyMonActualCatchRate` and `wEnemyMonBaseExp`: what a defeated enemy
/// is worth, which `GainExperience` divides in place.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpData {
    pub base_stats: [u8; NUM_STATS],
    pub catch_rate: u8,
    pub base_exp: u8,
}

/// A battle in progress, less the player's party, which stays on the world.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Battle {
    pub kind: BattleKind,
    pub player: Combatant,
    pub enemy: Combatant,
    /// `wPlayerMonNumber`: the party slot the player's mon is from.
    pub player_mon_number: u8,
    pub enemy_exp: ExpData,
    /// `wEnemyMons`: the trainer's party, or the wild mon alone.
    pub enemy_party: Vec<PartyMon>,
    /// `wDamage`, shared by both sides.
    pub damage: u16,
    pub critical_hit_or_ohko: CriticalHitOrOhko,
    pub move_missed: bool,
    /// `wMoveDidntMiss`: the move landed this turn, which spares a failed side effect its text.
    pub move_didnt_miss: bool,
    /// `wDamageMultipliers`: STAB in bit 7, the last effectiveness applied below it.
    pub damage_multipliers: u8,
    /// `wTrainerClass`, from 1; 0 in a wild battle.
    pub trainer_class: u8,
    /// `wAICount`: the trainer AI's uses left for this mon, `$FF` before its first turn.
    pub ai_count: u8,
    pub ai_layer2_encouragement: u8,
    /// `wPartyGainExpFlags`: a bit per party slot.
    pub gain_exp_flags: u8,
    /// `wPartyFoughtCurrentEnemyFlags`.
    pub fought_current_enemy_flags: u8,
    /// `wCanEvolveFlags`: the slots that gained a level, for `EvolutionAfterBattle`.
    pub can_evolve_flags: u8,
    /// `wMonIsDisobedient`: the player's mon is using a move it was not told to.
    pub mon_is_disobedient: bool,
    /// `wEscapedFromBattle`: Teleport, Roar or Whirlwind ended the battle.
    pub escaped_from_battle: bool,
    /// `wTotalPayDayMoney`, BCD.
    pub total_pay_day_money: [u8; 3],
    /// `wTransformedEnemyMonOriginalDVs`.
    pub transformed_enemy_original_dvs: Dvs,
    /// `wSafariBaitFactor` and `wSafariEscapeFactor`: turns left of eating and of anger.
    #[serde(default)]
    pub safari_bait_factor: u8,
    #[serde(default)]
    pub safari_escape_factor: u8,
}

impl Battle {
    /// `InitBattleVariables`: nothing volatile, the player's first mon on both sides until each is
    /// loaded over it.
    pub fn new(kind: BattleKind, player: &PartyMon, enemy_party: Vec<PartyMon>) -> Battle {
        Battle {
            kind,
            player: Combatant::new(BattleMon::from_party(player)),
            enemy: Combatant::new(BattleMon::from_party(player)),
            player_mon_number: 0,
            enemy_exp: ExpData::default(),
            enemy_party,
            damage: 0,
            critical_hit_or_ohko: CriticalHitOrOhko::Normal,
            move_missed: false,
            move_didnt_miss: false,
            damage_multipliers: 0,
            trainer_class: 0,
            ai_count: 0,
            ai_layer2_encouragement: 0,
            gain_exp_flags: 0,
            fought_current_enemy_flags: 0,
            can_evolve_flags: 0,
            mon_is_disobedient: false,
            escaped_from_battle: false,
            total_pay_day_money: [0; 3],
            transformed_enemy_original_dvs: Dvs::default(),
            safari_bait_factor: 0,
            safari_escape_factor: 0,
        }
    }

    pub fn side(&self, side: Side) -> &Combatant {
        match side {
            Side::Player => &self.player,
            Side::Enemy => &self.enemy,
        }
    }

    pub fn side_mut(&mut self, side: Side) -> &mut Combatant {
        match side {
            Side::Player => &mut self.player,
            Side::Enemy => &mut self.enemy,
        }
    }
}

/// What a battle routine reads beyond the battle: the player's party, `wPlayerID` and
/// `wObtainedBadges`. A harvested fixture carries one whole.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Arena {
    pub battle: Battle,
    pub party: Vec<PartyMon>,
    pub player_id: u16,
    pub badges: u8,
}

impl BattleMon {
    /// `LoadBattleMonFromParty` and `LoadEnemyMonFromParty`'s copy of the party struct's front.
    pub fn from_party(party_mon: &PartyMon) -> BattleMon {
        let mon = &party_mon.mon;
        BattleMon {
            species: mon.species,
            hp: mon.hp,
            party_pos: mon.box_level,
            status: mon.status,
            types: mon.types,
            catch_rate: mon.catch_rate,
            moves: mon.moves,
            dvs: mon.dvs,
            level: party_mon.level,
            stats: party_mon.stats,
            pp: mon.pp,
        }
    }
}

impl Combatant {
    /// A mon just sent out: its stats unmodified, nothing volatile.
    pub fn new(mon: BattleMon) -> Combatant {
        Combatant {
            unmodified_level: mon.level,
            unmodified_stats: mon.stats,
            stat_mods: [BASE_STAT_LEVEL; 6],
            status1: Status1::empty(),
            status2: Status2::empty(),
            status3: Status3::empty(),
            num_attacks_left: 0,
            confused_counter: 0,
            toxic_counter: 0,
            disabled_move: 0,
            bide_accumulated_damage: 0,
            substitute_hp: 0,
            selected_move: 0,
            move_list_index: 0,
            disabled_move_number: 0,
            used_move: 0,
            minimized: 0,
            current_move: MoveData { animation: 0, effect: 0, power: 0, move_type: 0, accuracy: 0, pp: 0 },
            mon,
        }
    }
}

impl Arena {
    /// What a battle fixture's input is laid over: the player's level 50 Tauros against a wild
    /// level 50 Rattata, both at full health with nothing volatile. Changing it invalidates every
    /// battle fixture.
    pub fn baseline() -> Arena {
        use crate::rng::GameRng;
        use crate::systems::add_mon::{new_party_mon, Origin};
        let mon = |species| new_party_mon(species, 50, 0, &Origin::Trainer, &mut GameRng::tape(vec![]));
        let (player, enemy) = (mon(PokemonSpecies::Tauros), mon(PokemonSpecies::Rattata));
        let mut battle = Battle::new(BattleKind::Wild, &player, vec![enemy.clone()]);
        battle.enemy = Combatant::new(BattleMon::from_party(&enemy));
        Arena {
            battle,
            party: vec![player],
            player_id: 0,
            badges: 0,
        }
    }
}

#[cfg(test)]
pub(crate) mod fixture {
    //! A battle fixture's output is what the routine changed: the fields of the arena after it that
    //! differ from before, nested as the arena is. Everything else must come out as it went in.

    use serde::de::DeserializeOwned;
    use serde::Serialize;
    use serde_json::Value;
    use crate::rng::GameRng;

    /// Lays `changed` over `value`, object by object. An array is changed element by element when
    /// `changed` names indices, and replaced whole when it is an array itself.
    pub fn apply(value: &mut Value, changed: &Value) {
        match (value, changed) {
            (Value::Object(into), Value::Object(fields)) => {
                for (key, field) in fields {
                    apply(into.get_mut(key).unwrap_or_else(|| panic!("no field {key}")), field);
                }
            }
            (Value::Array(into), Value::Object(elements)) => {
                for (index, element) in elements {
                    apply(&mut into[index.parse::<usize>().unwrap()], element);
                }
            }
            (value, changed) => *value = changed.clone(),
        }
    }

    /// A case's arena: the baseline with the input laid over it.
    pub fn arena(input: &Value) -> super::Arena {
        let mut value = serde_json::to_value(super::Arena::baseline()).unwrap();
        apply(&mut value, input);
        serde_json::from_value(value).unwrap()
    }

    /// `after` is `before` with `changed` laid over it; a `changed` of null is no change.
    pub fn assert_changed<T: Serialize + DeserializeOwned>(before: &T, after: &T, changed: &Value, context: &str) {
        let mut expected = serde_json::to_value(before).unwrap();
        if !changed.is_null() {
            apply(&mut expected, changed);
        }
        let actual = serde_json::to_value(after).unwrap();
        assert_eq!(actual, expected, "{context}");
    }

    /// Runs every case of a battle fixture: `run` gets the case's arena, its whole input and the
    /// tape, and answers what the routine returned, which must match along with every change and
    /// every random byte drawn.
    pub fn each_case(jsonl: &str, mut run: impl FnMut(&mut super::Arena, &Value, &mut GameRng) -> Value) {
        for (input, output, rng) in crate::fixtures::cases::<Value, Value>(jsonl) {
            let before = arena(&input["arena"]);
            let mut after = before.clone();
            let mut tape = GameRng::tape(rng);
            let returned = run(&mut after, &input, &mut tape);
            assert_changed(&before, &after, &output["changed"], &input.to_string());
            assert_eq!(returned, output.get("returned").cloned().unwrap_or(Value::Null), "{input}");
            let GameRng::Tape { bytes, cursor } = tape else { unreachable!() };
            assert_eq!(cursor, bytes.len(), "every random byte is drawn: {input}");
        }
    }

    pub fn side(input: &Value) -> super::Side {
        serde_json::from_value(input["side"].clone()).unwrap()
    }
}
