//! The battle routines, over the whole battle as WRAM lays it out: an `Arena` is written before
//! every call and read back after it, and a fixture keeps what the routine changed.
//!
//! A battle routine prints, animates and waits for frames between its arithmetic, and the oracle's
//! machine has its interrupts masked, so `run` returns straight out of the presentation routines a
//! case names instead of entering them, and records which it skipped.

mod ai;
mod damage;
mod effects;
mod experience;
mod safari;
mod start;
mod trainer;
mod turn;
#[cfg(feature = "slow-tests")]
mod generate;

use gb::cycles::MachineCycles;
use gb::game_boy::{Breakpoint, GameBoy, Stop};
use gb::ram::{RAM, ROM};
use poke_core::move_name::PokemonMoveName;
use poke_core::moves::MoveData;
use poke_core::species::PokemonSpecies;
use pokered::party::PartyMon;
use pokered::systems::battle::{Arena, Battle, BattleKind, BattleMon, Combatant, CriticalHitOrOhko, ExpData,
                               Status1, Status2, Status3};
use pokered::systems::stats::Dvs;
use serde_json::Value;
use crate::lockstep::breakpoint;
use crate::pokemon::symbols::{pokered_symbols as sym, DmgBank, DmgPointer};
use super::pokemon::{decode_party, encode_party};
use super::{Oracle, MBC_ROM_BANK, TRAP};

const BATTLE_STRUCT_LENGTH: usize = 29;
const PARTYMON_STRUCT_LENGTH: usize = 0x2C;
const LINK_STATE_NONE: u8 = 0;
const CONNECTION_NOT_ESTABLISHED: u8 = 0xFF;

fn oracle() -> Oracle {
    Oracle::from_state(include_bytes!("../../pokemon/data/at-celadon.bin"))
}

/// A label `pokered.sym` has and the generated symbols skip: a local one, with a dot in it.
fn local_label(label: &str) -> DmgPointer {
    include_str!("../../../../vendor/pokered/pokered.sym").lines()
        .find_map(|line| {
            let (at, name) = line.split_once(' ')?;
            let (bank, address) = at.split_once(':')?;
            (name == label).then(|| DmgPointer {
                bank: DmgBank::ROM { bank: u8::from_str_radix(bank, 16).unwrap() },
                address: u16::from_str_radix(address, 16).unwrap(),
            })
        })
        .unwrap_or_else(|| panic!("no {label} in pokered.sym"))
}

/// The global label at `address`, as the ROM bank mapped there now places it.
fn label_at(gb: &GameBoy, address: u16) -> Option<&'static str> {
    use std::collections::HashMap;
    use std::sync::OnceLock;
    static LABELS: OnceLock<HashMap<(u8, u16), &'static str>> = OnceLock::new();
    let labels = LABELS.get_or_init(|| include_str!("../../../../vendor/pokered/pokered.sym").lines()
        .filter_map(|line| {
            let (at, name) = line.split_once(' ')?;
            let (bank, address) = at.split_once(':')?;
            if name.contains('.') {
                return None;
            }
            Some(((u8::from_str_radix(bank, 16).ok()?, u16::from_str_radix(address, 16).ok()?), name))
        })
        .collect());
    let bank = if (0x4000..0x8000).contains(&address) { gb.core().mmu().rom_bank() as u8 } else { 0 };
    labels.get(&(bank, address)).copied()
}

/// What the battle routines call to print, animate, draw and wait: never entered by a harvest.
const PRESENTATION: [(&str, DmgPointer); 12] = [
    ("PrintText", sym::PrintText),
    ("MoveAnimation", sym::MoveAnimation),
    ("HideSubstituteShowMonAnim", sym::HideSubstituteShowMonAnim),
    ("ReshowSubstituteAnim", sym::ReshowSubstituteAnim),
    ("DelayFrames", sym::DelayFrames),
    ("PlaySoundWaitForCurrent", sym::PlaySoundWaitForCurrent),
    ("WaitForSoundToFinish", sym::WaitForSoundToFinish),
    ("UpdateHPBar2", sym::UpdateHPBar2),
    ("DrawHUDsAndHPBars", sym::DrawHUDsAndHPBars),
    ("DrawPlayerHUDAndHPBar", sym::DrawPlayerHUDAndHPBar),
    ("DrawEnemyHUDAndHPBar", sym::DrawEnemyHUDAndHPBar),
    ("AIPrintItemUse_", sym::AIPrintItemUse_),
];

/// The texts printed on the way, by label, as `PrintText` is called with each.
fn text_printed(texts: &mut Vec<&'static str>, name: &str, gb: &GameBoy) {
    if name == "PrintText" {
        let registers = gb.core().registers();
        let hl = u16::from_be_bytes([registers.h, registers.l]);
        texts.push(label_at(gb, hl).unwrap_or_else(|| panic!("no label at {hl:#06x}")));
    }
}

/// Where a run ended, and what it went through on the way.
#[derive(Debug)]
struct Ran {
    rng: Vec<u8>,
    /// The presentation routines returned from unentered, by name, in order.
    skipped: Vec<&'static str>,
    /// The stop reached, or `None` for a return.
    exit: Option<&'static str>,
}

/// `entry` with the registers as set, until it returns or reaches one of `stops`, returning out of
/// every one of `skips` as soon as it is called.
fn run(oracle: &mut Oracle, entry: DmgPointer, stops: &[(&'static str, DmgPointer)],
       skips: &[(&'static str, DmgPointer)], observe: &mut dyn FnMut(&'static str, &mut GameBoy)) -> Ran {
    let gb = &mut oracle.gb;
    let mmu = gb.core_mut().mmu_mut();
    if let DmgBank::ROM { bank } = entry.bank && entry.address >= 0x4000 {
        mmu.write(MBC_ROM_BANK, bank);
        mmu.write(sym::hLoadedROMBank.address, bank);
    }
    let sp = oracle.stack - 2;
    mmu.write_u16_le(sp, TRAP);
    let registers = gb.core_mut().registers_mut();
    registers.sp = sp;
    registers.pc = entry.address;

    let random = breakpoint(sym::Random);
    let trap = Breakpoint::new(0, TRAP);
    let named: Vec<(&'static str, Breakpoint, bool)> = stops.iter().map(|&(name, at)| (name, breakpoint(at), true))
        .chain(skips.iter().map(|&(name, at)| (name, breakpoint(at), false)))
        .collect();
    let mut points = vec![trap, random];
    points.extend(named.iter().map(|&(_, point, _)| point));
    let budget = MachineCycles::PER_FRAME * 600;
    let mut ran = Ran { rng: vec![], skipped: vec![], exit: None };
    let mut hit = gb.run_until(&points, budget).0;
    loop {
        let point = match hit {
            Stop::Breakpoint(point) => point,
            stop => panic!("{entry} did not return: {stop:?}"),
        };
        if point == trap {
            return ran;
        }
        if point == random {
            assert!(ran.rng.len() < 10_000, "{entry} draws random bytes for ever");
            let (stop, _) = gb.run_to_return(budget);
            assert!(matches!(stop, Stop::Returned { .. }), "Random did not return: {stop:?}");
            ran.rng.push(gb.core().registers().a);
        } else {
            let &(name, _, stop) = named.iter().find(|&&(_, at, _)| at == point).unwrap();
            if stop {
                ran.exit = Some(name);
                return ran;
            }
            ran.skipped.push(name);
            observe(name, gb);
            let sp = gb.core().registers().sp;
            let return_address = gb.core().mmu().read_u16_le(sp);
            let registers = gb.core_mut().registers_mut();
            registers.sp = sp + 2;
            registers.pc = return_address;
        }
        let pc = gb.core().registers().pc;
        let bank = gb.core().mmu().rom_bank();
        // `run_until` steps before it looks, so a return that lands on a point is caught here.
        hit = match points.iter().find(|p| p.address == pc && (!(0x4000..0x8000).contains(&pc) || p.bank as usize == bank)) {
            Some(&point) => Stop::Breakpoint(point),
            None => gb.run_until(&points, budget).0,
        };
    }
}

fn be16(bytes: &[u8]) -> u16 {
    u16::from_be_bytes([bytes[0], bytes[1]])
}

fn move_bytes(moves: [Option<PokemonMoveName>; 4]) -> [u8; 4] {
    moves.map(|m| m.map_or(0, |m| m as u8))
}

fn moves_of(bytes: &[u8]) -> [Option<PokemonMoveName>; 4] {
    [0, 1, 2, 3].map(|i| (bytes[i] != 0).then(|| PokemonMoveName::from_repr(bytes[i]).expect("a move")))
}

/// Every address one side of the battle keeps.
struct SideAt {
    mon: DmgPointer,
    unmodified_level: DmgPointer,
    stat_mods: DmgPointer,
    status1: DmgPointer,
    num_attacks_left: DmgPointer,
    confused_counter: DmgPointer,
    toxic_counter: DmgPointer,
    disabled_move: DmgPointer,
    bide_accumulated_damage: DmgPointer,
    substitute_hp: DmgPointer,
    selected_move: DmgPointer,
    move_list_index: DmgPointer,
    disabled_move_number: DmgPointer,
    used_move: DmgPointer,
    minimized: DmgPointer,
    move_num: DmgPointer,
}

const PLAYER: SideAt = SideAt {
    mon: sym::wBattleMon,
    unmodified_level: sym::wPlayerMonUnmodifiedLevel,
    stat_mods: sym::wPlayerMonStatMods,
    status1: sym::wPlayerBattleStatus1,
    num_attacks_left: sym::wPlayerNumAttacksLeft,
    confused_counter: sym::wPlayerConfusedCounter,
    toxic_counter: sym::wPlayerToxicCounter,
    disabled_move: sym::wPlayerDisabledMove,
    bide_accumulated_damage: sym::wPlayerBideAccumulatedDamage,
    substitute_hp: sym::wPlayerSubstituteHP,
    selected_move: sym::wPlayerSelectedMove,
    move_list_index: sym::wPlayerMoveListIndex,
    disabled_move_number: sym::wPlayerDisabledMoveNumber,
    used_move: sym::wPlayerUsedMove,
    minimized: sym::wPlayerMonMinimized,
    move_num: sym::wPlayerMoveNum,
};

const ENEMY: SideAt = SideAt {
    mon: sym::wEnemyMon,
    unmodified_level: sym::wEnemyMonUnmodifiedLevel,
    stat_mods: sym::wEnemyMonStatMods,
    status1: sym::wEnemyBattleStatus1,
    num_attacks_left: sym::wEnemyNumAttacksLeft,
    confused_counter: sym::wEnemyConfusedCounter,
    toxic_counter: sym::wEnemyToxicCounter,
    disabled_move: sym::wEnemyDisabledMove,
    bide_accumulated_damage: sym::wEnemyBideAccumulatedDamage,
    substitute_hp: sym::wEnemySubstituteHP,
    selected_move: sym::wEnemySelectedMove,
    move_list_index: sym::wEnemyMoveListIndex,
    disabled_move_number: sym::wEnemyDisabledMoveNumber,
    used_move: sym::wEnemyUsedMove,
    minimized: sym::wEnemyMonMinimized,
    move_num: sym::wEnemyMoveNum,
};

fn encode_battle_mon(mon: &BattleMon) -> Vec<u8> {
    let mut bytes = vec![mon.species as u8];
    bytes.extend(mon.hp.to_be_bytes());
    bytes.extend([mon.party_pos, mon.status, mon.types[0], mon.types[1], mon.catch_rate]);
    bytes.extend(move_bytes(mon.moves));
    bytes.extend(mon.dvs.0);
    bytes.push(mon.level);
    bytes.extend(mon.stats.iter().flat_map(|stat| stat.to_be_bytes()));
    bytes.extend(mon.pp);
    assert_eq!(bytes.len(), BATTLE_STRUCT_LENGTH);
    bytes
}

fn decode_battle_mon(b: &[u8]) -> BattleMon {
    BattleMon {
        species: PokemonSpecies::from_repr(b[0]).unwrap_or_else(|| panic!("species {:#04x}", b[0])),
        hp: be16(&b[1..]),
        party_pos: b[3],
        status: b[4],
        types: [b[5], b[6]],
        catch_rate: b[7],
        moves: moves_of(&b[8..12]),
        dvs: Dvs([b[12], b[13]]),
        level: b[14],
        stats: [0, 1, 2, 3, 4].map(|i| be16(&b[15 + 2 * i..])),
        pp: b[25..29].try_into().unwrap(),
    }
}

fn encode_move(data: &MoveData) -> [u8; 6] {
    [data.animation, data.effect, data.power, data.move_type, data.accuracy, data.pp]
}

fn write_side(oracle: &mut Oracle, at: &SideAt, side: &Combatant) {
    oracle.write(at.mon, &encode_battle_mon(&side.mon));
    let mut unmodified = vec![side.unmodified_level];
    unmodified.extend(side.unmodified_stats.iter().flat_map(|stat| stat.to_be_bytes()));
    oracle.write(at.unmodified_level, &unmodified);
    oracle.write(at.stat_mods, &side.stat_mods);
    oracle.write(at.status1, &[side.status1.bits(), side.status2.bits(), side.status3.bits()]);
    oracle.write(at.num_attacks_left, &[side.num_attacks_left]);
    oracle.write(at.confused_counter, &[side.confused_counter]);
    oracle.write(at.toxic_counter, &[side.toxic_counter]);
    oracle.write(at.disabled_move, &[side.disabled_move]);
    oracle.write(at.bide_accumulated_damage, &side.bide_accumulated_damage.to_be_bytes());
    oracle.write(at.substitute_hp, &[side.substitute_hp]);
    oracle.write(at.selected_move, &[side.selected_move]);
    oracle.write(at.move_list_index, &[side.move_list_index]);
    oracle.write(at.disabled_move_number, &[side.disabled_move_number]);
    oracle.write(at.used_move, &[side.used_move]);
    oracle.write(at.minimized, &[side.minimized]);
    oracle.write(at.move_num, &encode_move(&side.current_move));
}

fn read_side(oracle: &Oracle, at: &SideAt) -> Combatant {
    let byte = |pointer: DmgPointer| oracle.read(pointer, 1)[0];
    let unmodified = oracle.read(at.unmodified_level, 11);
    let statuses = oracle.read(at.status1, 3);
    let current = oracle.read(at.move_num, 6);
    Combatant {
        mon: decode_battle_mon(&oracle.read(at.mon, BATTLE_STRUCT_LENGTH)),
        unmodified_level: unmodified[0],
        unmodified_stats: [0, 1, 2, 3, 4].map(|i| be16(&unmodified[1 + 2 * i..])),
        stat_mods: oracle.read(at.stat_mods, 6).try_into().unwrap(),
        status1: Status1::from_bits_retain(statuses[0]),
        status2: Status2::from_bits_retain(statuses[1]),
        status3: Status3::from_bits_retain(statuses[2]),
        num_attacks_left: byte(at.num_attacks_left),
        confused_counter: byte(at.confused_counter),
        toxic_counter: byte(at.toxic_counter),
        disabled_move: byte(at.disabled_move),
        bide_accumulated_damage: be16(&oracle.read(at.bide_accumulated_damage, 2)),
        substitute_hp: byte(at.substitute_hp),
        selected_move: byte(at.selected_move),
        move_list_index: byte(at.move_list_index),
        disabled_move_number: byte(at.disabled_move_number),
        used_move: byte(at.used_move),
        minimized: byte(at.minimized),
        current_move: MoveData {
            animation: current[0], effect: current[1], power: current[2], move_type: current[3],
            accuracy: current[4], pp: current[5],
        },
    }
}

fn write_party(oracle: &mut Oracle, count: DmgPointer, species: DmgPointer, mons: DmgPointer, party: &[PartyMon]) {
    oracle.write(count, &[party.len() as u8]);
    let mut list: Vec<u8> = party.iter().map(|mon| mon.mon.species as u8).collect();
    list.push(0xFF);
    oracle.write(species, &list);
    for (slot, mon) in party.iter().enumerate() {
        oracle.write(mons + (slot * PARTYMON_STRUCT_LENGTH) as u16, &encode_party(mon));
    }
}

fn read_party(oracle: &Oracle, count: DmgPointer, mons: DmgPointer) -> Vec<PartyMon> {
    (0..oracle.read(count, 1)[0] as usize)
        .map(|slot| decode_party(&oracle.read(mons + (slot * PARTYMON_STRUCT_LENGTH) as u16, PARTYMON_STRUCT_LENGTH)))
        .collect()
}

/// The arena into WRAM, with the machine outside a link battle.
fn write_arena(oracle: &mut Oracle, arena: &Arena) {
    let battle = &arena.battle;
    oracle.write(sym::wLinkState, &[LINK_STATE_NONE]);
    oracle.write(sym::hSerialConnectionStatus, &[CONNECTION_NOT_ESTABLISHED]);
    oracle.write(sym::wPlayerStatsToDouble, &[0, 0]);
    oracle.write(sym::wEnemyStatsToDouble, &[0, 0]);
    write_side(oracle, &PLAYER, &battle.player);
    write_side(oracle, &ENEMY, &battle.enemy);
    oracle.write(sym::wIsInBattle, &[match battle.kind {
        BattleKind::Lost => 0xFF,
        BattleKind::Wild => 1,
        BattleKind::Trainer => 2,
    }]);
    oracle.write(sym::wPlayerMonNumber, &[battle.player_mon_number]);
    let exp = &battle.enemy_exp;
    oracle.write(sym::wEnemyMonBaseStats, &exp.base_stats);
    oracle.write(sym::wEnemyMonActualCatchRate, &[exp.catch_rate, exp.base_exp]);
    write_party(oracle, sym::wEnemyPartyCount, sym::wEnemyPartySpecies, sym::wEnemyMons, &battle.enemy_party);
    oracle.write(sym::wDamage, &battle.damage.to_be_bytes());
    oracle.write(sym::wCriticalHitOrOHKO, &[match battle.critical_hit_or_ohko {
        CriticalHitOrOhko::Normal => 0,
        CriticalHitOrOhko::CriticalHit => 1,
        CriticalHitOrOhko::SuccessfulOhko => 2,
        CriticalHitOrOhko::FailedOhko => 0xFF,
    }]);
    oracle.write(sym::wMoveMissed, &[battle.move_missed as u8]);
    oracle.write(sym::wMoveDidntMiss, &[battle.move_didnt_miss as u8]);
    oracle.write(sym::wDamageMultipliers, &[battle.damage_multipliers]);
    oracle.write(sym::wTrainerClass, &[battle.trainer_class]);
    oracle.write(sym::wAICount, &[battle.ai_count]);
    oracle.write(sym::wAILayer2Encouragement, &[battle.ai_layer2_encouragement]);
    oracle.write(sym::wPartyGainExpFlags, &[battle.gain_exp_flags]);
    oracle.write(sym::wPartyFoughtCurrentEnemyFlags, &[battle.fought_current_enemy_flags]);
    oracle.write(sym::wCanEvolveFlags, &[battle.can_evolve_flags]);
    oracle.write(sym::wEscapedFromBattle, &[battle.escaped_from_battle as u8]);
    oracle.write(sym::wMonIsDisobedient, &[battle.mon_is_disobedient as u8]);
    oracle.write(sym::wTotalPayDayMoney, &battle.total_pay_day_money);
    oracle.write(sym::wTransformedEnemyMonOriginalDVs, &battle.transformed_enemy_original_dvs.0);
    oracle.write(sym::wSafariEscapeFactor, &[battle.safari_escape_factor, battle.safari_bait_factor]);
    // `wCurEnemyLevel` is the enemy mon's level whenever a battle routine reads it.
    oracle.write(sym::wCurEnemyLevel, &[battle.enemy.mon.level]);
    write_party(oracle, sym::wPartyCount, sym::wPartySpecies, sym::wPartyMons, &arena.party);
    oracle.write(sym::wPlayerID, &arena.player_id.to_be_bytes());
    oracle.write(sym::wObtainedBadges, &[arena.badges]);
}

fn read_arena(oracle: &Oracle) -> Arena {
    let byte = |pointer: DmgPointer| oracle.read(pointer, 1)[0];
    let exp = oracle.read(sym::wEnemyMonBaseStats, 7);
    Arena {
        battle: Battle {
            kind: match byte(sym::wIsInBattle) {
                0xFF => BattleKind::Lost,
                1 => BattleKind::Wild,
                2 => BattleKind::Trainer,
                other => panic!("wIsInBattle {other}"),
            },
            player: read_side(oracle, &PLAYER),
            enemy: read_side(oracle, &ENEMY),
            player_mon_number: byte(sym::wPlayerMonNumber),
            enemy_exp: ExpData { base_stats: exp[..5].try_into().unwrap(), catch_rate: exp[5], base_exp: exp[6] },
            enemy_party: read_party(oracle, sym::wEnemyPartyCount, sym::wEnemyMons),
            damage: be16(&oracle.read(sym::wDamage, 2)),
            critical_hit_or_ohko: match byte(sym::wCriticalHitOrOHKO) {
                0 => CriticalHitOrOhko::Normal,
                1 => CriticalHitOrOhko::CriticalHit,
                2 => CriticalHitOrOhko::SuccessfulOhko,
                0xFF => CriticalHitOrOhko::FailedOhko,
                other => panic!("wCriticalHitOrOHKO {other}"),
            },
            move_missed: match byte(sym::wMoveMissed) {
                0 => false,
                1 => true,
                other => panic!("wMoveMissed {other}"),
            },
            move_didnt_miss: match byte(sym::wMoveDidntMiss) {
                0 => false,
                1 => true,
                other => panic!("wMoveDidntMiss {other}"),
            },
            damage_multipliers: byte(sym::wDamageMultipliers),
            trainer_class: byte(sym::wTrainerClass),
            ai_count: byte(sym::wAICount),
            ai_layer2_encouragement: byte(sym::wAILayer2Encouragement),
            gain_exp_flags: byte(sym::wPartyGainExpFlags),
            fought_current_enemy_flags: byte(sym::wPartyFoughtCurrentEnemyFlags),
            can_evolve_flags: byte(sym::wCanEvolveFlags),
            mon_is_disobedient: match byte(sym::wMonIsDisobedient) {
                0 => false,
                1 => true,
                other => panic!("wMonIsDisobedient {other}"),
            },
            escaped_from_battle: match byte(sym::wEscapedFromBattle) {
                0 => false,
                1 => true,
                other => panic!("wEscapedFromBattle {other}"),
            },
            total_pay_day_money: oracle.read(sym::wTotalPayDayMoney, 3).try_into().unwrap(),
            transformed_enemy_original_dvs: Dvs(oracle.read(sym::wTransformedEnemyMonOriginalDVs, 2).try_into().unwrap()),
            safari_escape_factor: byte(sym::wSafariEscapeFactor),
            safari_bait_factor: byte(sym::wSafariBaitFactor),
        },
        party: read_party(oracle, sym::wPartyCount, sym::wPartyMons),
        player_id: be16(&oracle.read(sym::wPlayerID, 2)),
        badges: byte(sym::wObtainedBadges),
    }
}

/// What differs from `before` in `after`, nested as they are: an array of the same length by the
/// indices that changed, anything else whole. `None` when nothing did.
fn changed(before: &Value, after: &Value) -> Option<Value> {
    if before == after {
        return None;
    }
    Some(match (before, after) {
        (Value::Object(before), Value::Object(after)) => Value::Object(after.iter()
            .filter_map(|(key, value)| changed(&before[key], value).map(|change| (key.clone(), change)))
            .collect()),
        (Value::Array(before), Value::Array(after)) if before.len() == after.len() => Value::Object(after.iter()
            .enumerate()
            .filter_map(|(index, value)| changed(&before[index], value).map(|change| (index.to_string(), change)))
            .collect()),
        _ => after.clone(),
    })
}

/// The arena as a fixture's input stores it: what differs from the baseline.
fn sparse(arena: &Arena) -> Value {
    let baseline = serde_json::to_value(Arena::baseline()).unwrap();
    changed(&baseline, &serde_json::to_value(arena).unwrap()).unwrap_or(Value::Object(Default::default()))
}

/// Writes `arena`, runs `entry` and reads the arena back: the change as a fixture's output keeps it.
fn run_arena(oracle: &mut Oracle, arena: &Arena, entry: DmgPointer, stops: &[(&'static str, DmgPointer)],
             skips: &[(&'static str, DmgPointer)]) -> (Arena, Value, Ran) {
    run_arena_observed(oracle, arena, entry, stops, skips, &mut |_, _| {})
}

/// `run_arena`, with `observe` handed the machine at every skip, to read or to answer for what it
/// skipped.
fn run_arena_observed(oracle: &mut Oracle, arena: &Arena, entry: DmgPointer, stops: &[(&'static str, DmgPointer)],
                      skips: &[(&'static str, DmgPointer)], observe: &mut dyn FnMut(&'static str, &mut GameBoy))
                      -> (Arena, Value, Ran) {
    write_arena(oracle, arena);
    assert_eq!(&read_arena(oracle), arena, "the arena round-trips through WRAM");
    let ran = run(oracle, entry, stops, skips, observe);
    let after = read_arena(oracle);
    let change = changed(&serde_json::to_value(arena).unwrap(), &serde_json::to_value(&after).unwrap())
        .unwrap_or(Value::Null);
    (after, change, ran)
}

#[test]
fn the_baseline_round_trips_through_wram() {
    let mut oracle = oracle();
    let arena = Arena::baseline();
    write_arena(&mut oracle, &arena);
    assert_eq!(read_arena(&oracle), arena);
    assert_eq!(sparse(&arena), Value::Object(Default::default()));
}

#[test]
fn a_change_lists_only_what_moved() {
    let mut arena = Arena::baseline();
    let before = serde_json::to_value(&arena).unwrap();
    arena.battle.player.mon.stats[2] = 7;
    arena.battle.damage = 12;
    let change = changed(&before, &serde_json::to_value(&arena).unwrap()).unwrap();
    assert_eq!(change, serde_json::json!({"battle": {"player": {"mon": {"stats": {"2": 7}}}, "damage": 12}}));
}
