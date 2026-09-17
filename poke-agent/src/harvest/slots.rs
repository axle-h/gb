//! The slot machine on the cartridge: the flags it rolls before the wheels move, the symbols each
//! wheel offset shows, a spinning frame with its objects, the match search and what a win pays.

use poke_core::rom_gfx::rom_slice;
use poke_core::symbols::pokered_local_labels::SlotMachine_CheckForMatches as search;
use pokered::gfx::layers::Object;
use pokered::systems::slots::{Matches, Reward, Wheels};
use crate::pokemon::symbols::{pokered_symbols as sym, DmgBank, DmgPointer};
use super::Oracle;

fn oracle() -> Oracle {
    Oracle::from_state(include_bytes!("../pokemon/data/at-celadon.bin"))
}

/// `Random` adds `rDIV` to `hRandomAdd`, so seeding that byte walks the value it returns without
/// setting it: a sweep of seeds is a sweep of rolls, and what it actually rolled is on the tape.
fn seed(oracle: &mut Oracle, seed: u8) {
    oracle.write(sym::hRandomAdd, &[seed]);
}

/// `SlotMachine_SetFlags` with `wSlotMachineFlags`, `wSlotMachineAllowMatchesCounter` and
/// `wSlotMachineSevenAndBarModeChance` as given.
fn set_flags(oracle: &mut Oracle, flags: u8, counter: u8, chance: u8) -> ((u8, u8), Vec<u8>) {
    oracle.write(sym::wSlotMachineFlags, &[flags]);
    oracle.write(sym::wSlotMachineAllowMatchesCounter, &[counter]);
    oracle.write(sym::wSlotMachineSevenAndBarModeChance, &[chance]);
    let called = oracle.call(sym::SlotMachine_SetFlags);
    let out = (oracle.read(sym::wSlotMachineFlags, 1)[0], oracle.read(sym::wSlotMachineAllowMatchesCounter, 1)[0]);
    (out, called.rng)
}

fn write_offsets(oracle: &mut Oracle, offsets: [u8; 3]) {
    oracle.write(sym::wSlotMachineWheel1Offset, &offsets);
}

/// `SlotMachine_GetWheel3Tiles`, which falls through wheels 2 and 1 and so fills all nine.
fn wheel_tiles(oracle: &mut Oracle, offsets: [u8; 3]) -> [[u8; 3]; 3] {
    write_offsets(oracle, offsets);
    oracle.call(sym::SlotMachine_GetWheel3Tiles);
    let tiles = oracle.read(sym::wSlotMachineWheel1BottomTile, 9);
    std::array::from_fn(|w| std::array::from_fn(|i| tiles[w * 3 + i]))
}

fn objects(oracle: &Oracle) -> Vec<Object> {
    oracle.read(sym::wShadowOAMSprite00, 36 * 4).chunks_exact(4)
        .map(|o| Object { y: o[0], x: o[1], tile: o[2], attributes: o[3] })
        .collect()
}

/// One pass of the three `SlotMachine_StopOrAnimWheel` routines, as `SlotMachine_SpinWheels` makes
/// it: the wheels after it, and whether wheel 3 came to rest.
fn stop_or_anim_wheels(oracle: &mut Oracle, wheels: Wheels, stopping: u8, flags: u8) -> (Wheels, bool) {
    write_offsets(oracle, wheels.offsets);
    oracle.write(sym::wSlotMachineWheel1SlipCounter, &wheels.slip);
    oracle.write(sym::wStoppingWhichSlotMachineWheel, &[stopping]);
    oracle.write(sym::wSlotMachineFlags, &[flags]);
    oracle.call(sym::SlotMachine_StopOrAnimWheel1);
    oracle.call(sym::SlotMachine_StopOrAnimWheel2);
    oracle.call(sym::SlotMachine_StopOrAnimWheel3);
    let stopped = oracle.registers().flags.c;
    let after = Wheels {
        offsets: oracle.read(sym::wSlotMachineWheel1Offset, 3).try_into().unwrap(),
        slip: oracle.read(sym::wSlotMachineWheel1SlipCounter, 2).try_into().unwrap(),
    };
    (after, stopped)
}

/// `SlotMachine_AnimWheel1`..`3`: the thirty-six objects the three wheels are drawn from, and the
/// offsets they leave behind.
fn anim_wheels(oracle: &mut Oracle, offsets: [u8; 3]) -> (Vec<Object>, [u8; 3]) {
    write_offsets(oracle, offsets);
    oracle.call(sym::SlotMachine_AnimWheel1);
    oracle.call(sym::SlotMachine_AnimWheel2);
    oracle.call(sym::SlotMachine_AnimWheel3);
    (objects(oracle), oracle.read(sym::wSlotMachineWheel1Offset, 3).try_into().unwrap())
}

/// One pass of `SlotMachine_CheckForMatches`, stopped at whichever of its three answers it reaches:
/// the roll and the miss both go on to wait for frames, and the win has its symbol in `hl` before
/// it writes it anywhere.
fn check_for_matches(oracle: &mut Oracle, offsets: [u8; 3], bet: u8, flags: u8, reroll: u8) -> (Matches, u8) {
    write_offsets(oracle, offsets);
    oracle.write(sym::wSlotMachineBet, &[bet]);
    oracle.write(sym::wSlotMachineFlags, &[flags]);
    oracle.write(sym::wSlotMachineRerollCounter, &[reroll]);
    let stops = [search::noMatch, search::rollWheel3DownByOneSymbol, search::acceptMatch];
    let (_, stopped) = oracle.call_until(sym::SlotMachine_CheckForMatches, &stops);
    let found = match stopped.expect("the match search never answered") {
        stop if stop == search::acceptMatch => {
            let registers = oracle.registers();
            let at = DmgPointer { bank: DmgBank::WRAM, address: u16::from_le_bytes([registers.l, registers.h]) };
            Matches::Won(oracle.read(at, 1)[0])
        }
        stop if stop == search::noMatch => Matches::Lost,
        _ => Matches::Reroll,
    };
    (found, oracle.read(sym::wSlotMachineRerollCounter, 1)[0])
}

/// `ld hl, YeahText` and the `PrintText` after it, which the three-seven reward is entered past
/// because it waits for frames.
const YEAH_TEXT: u16 = 6;

/// The `SlotRewardPointers` function for a symbol, indexed as the cartridge indexes it.
fn slot_reward(oracle: &mut Oracle, symbol: u8, flags: u8, counter: u8) -> ((Reward, u8, u8), Vec<u8>) {
    oracle.write(sym::wSlotMachineFlags, &[flags]);
    oracle.write(sym::wSlotMachineAllowMatchesCounter, &[counter]);
    let pointers = rom_slice(sym::SlotRewardPointers);
    let at = (symbol - 2) as usize;
    let mut entry = sym::SlotReward300Func;
    entry.address = u16::from_le_bytes([pointers[at], pointers[at + 1]]);
    if entry.address == sym::SlotReward300Func.address {
        entry.address += YEAH_TEXT;
    }
    let called = oracle.call(entry);
    let registers = oracle.registers();
    let reward = Reward { coins: u16::from_be_bytes([registers.d, registers.e]), flashes: registers.b };
    let out = (reward, oracle.read(sym::wSlotMachineFlags, 1)[0], oracle.read(sym::wSlotMachineAllowMatchesCounter, 1)[0]);
    (out, called.rng)
}

/// `GameCornerSelectLuckySlotMachine`, which the Game Corner runs each time it loads.
fn lucky_slot_machine(oracle: &mut Oracle) -> (u8, Vec<u8>) {
    oracle.write(sym::wCurrentMapScriptFlags, &[0xFF]);
    let called = oracle.call(sym::GameCornerSelectLuckySlotMachine);
    (oracle.read(sym::wLuckySlotHiddenEventIndex, 1)[0], called.rng)
}

#[test]
fn a_wheel_only_ever_rests_on_an_odd_offset() {
    let mut oracle = oracle();
    let wheels = Wheels { offsets: [0x1C, 0x1C, 0x1C], slip: [0, 0] };
    let (_, stopped) = stop_or_anim_wheels(&mut oracle, wheels, 3, 0);
    assert!(!stopped, "$1c is even, so no wheel may stop on it");
    let odd = Wheels { offsets: [0x1D, 0x1D, 0x1D], slip: [0, 0] };
    let (after, stopped) = stop_or_anim_wheels(&mut oracle, odd, 3, 0);
    assert!(stopped);
    assert_eq!(after.offsets, [0x1D, 0x1D, 0x1D], "a wheel that has stopped does not step");
}

/// Three sevens on a one-coin bet, which the flags have to allow before they are paid.
#[test]
fn a_line_the_flags_forbid_is_rolled_away_rather_than_paid() {
    use pokered::systems::slots::{wheel, CAN_WIN_WITH_7_OR_BAR, SEVEN};
    let mut oracle = oracle();
    // A middle row of sevens: each wheel's offset put one below its first seven.
    let offsets: [u8; 3] = std::array::from_fn(|w| {
        (1..30).step_by(2).find(|&o| wheel(w)[o + 2] == SEVEN).expect("a seven in reach") as u8
    });
    assert_eq!(check_for_matches(&mut oracle, offsets, 1, 0, 4), (Matches::Reroll, 4));
    assert_eq!(check_for_matches(&mut oracle, offsets, 1, CAN_WIN_WITH_7_OR_BAR, 4), (Matches::Won(SEVEN), 4));
}

#[test]
fn a_miss_spends_the_reroll_counter_and_then_gives_up() {
    use pokered::systems::slots::CAN_WIN;
    let mut oracle = oracle();
    // The middle row here is a mouse, a fish and a bird, which is no line at any bet.
    let offsets = [1, 1, 1];
    assert_eq!(check_for_matches(&mut oracle, offsets, 1, CAN_WIN, 4), (Matches::Reroll, 3));
    assert_eq!(check_for_matches(&mut oracle, offsets, 1, CAN_WIN, 1), (Matches::Lost, 0));
    assert_eq!(check_for_matches(&mut oracle, offsets, 1, 0, 4), (Matches::Lost, 4),
        "a player who may not win is told so without the counter being spent");
}

#[test]
#[cfg(feature = "slow-tests")]
#[ignore = "a tool: writes pokered/fixtures/slots/*.jsonl under GB_REGEN_FIXTURES=1"]
fn harvest_slots() {
    use pokered::systems::slots::{BAR, CAN_WIN, CAN_WIN_WITH_7_OR_BAR, CHERRY, BIRD, FISH, MOUSE, LUCKY, NOT_LUCKY, SEVEN, WHEEL_WRAP};
    use rand::{RngExt, SeedableRng};
    use serde::Serialize;
    use super::{write_fixture, Case};
    let mut oracle = oracle();
    let mut rng = rand::rngs::StdRng::seed_from_u64(0x5107);

    #[derive(Serialize)]
    struct FlagsCase { flags: u8, allow_matches_counter: u8, chance: u8 }
    let mut cases = Vec::new();
    for flags in [0, CAN_WIN, CAN_WIN_WITH_7_OR_BAR] {
        for counter in [0, 1, 60] {
            for chance in [NOT_LUCKY, LUCKY] {
                for byte in 0..=255u8 {
                    seed(&mut oracle, byte);
                    let input = FlagsCase { flags, allow_matches_counter: counter, chance };
                    let (output, rng) = set_flags(&mut oracle, flags, counter, chance);
                    cases.push(Case { input, output, rng });
                }
            }
        }
    }
    let rolls: std::collections::BTreeSet<u8> = cases.iter().filter_map(|case| case.rng.first().copied()).collect();
    assert!(rolls.len() > 200, "only {} of the 256 rolls were reached", rolls.len());
    write_fixture("slots", "set_flags", &cases);

    let every_offset: Vec<[u8; 3]> = (0..WHEEL_WRAP)
        .flat_map(|a| (0..WHEEL_WRAP).map(move |b| [a, b, (a + 2 * b) % WHEEL_WRAP]))
        .collect();
    let cases: Vec<_> = every_offset.iter().map(|&offsets| Case {
        input: offsets, output: wheel_tiles(&mut oracle, offsets), rng: vec![],
    }).collect();
    write_fixture("slots", "wheel_tiles", &cases);

    #[derive(Serialize)]
    struct SpinCase { wheels: Wheels, stopping: u8, flags: u8 }
    #[derive(Serialize)]
    struct SpinOutput { wheels: Wheels, stopped: bool }
    let mut cases = Vec::new();
    for &offsets in every_offset.iter().step_by(3) {
        for stopping in 0..4u8 {
            for flags in [0, CAN_WIN, CAN_WIN_WITH_7_OR_BAR] {
                let slip = [rng.random_range(0..=4), rng.random_range(0..=4)];
                let wheels = Wheels { offsets, slip };
                let (after, stopped) = stop_or_anim_wheels(&mut oracle, wheels, stopping, flags);
                cases.push(Case {
                    input: SpinCase { wheels, stopping, flags },
                    output: SpinOutput { wheels: after, stopped },
                    rng: vec![],
                });
            }
        }
    }
    let stopped = cases.iter().filter(|case| case.output.stopped).count();
    assert!(stopped > 50, "only {stopped} spins ended among {}", cases.len());
    write_fixture("slots", "stop_or_anim_wheels", &cases);

    #[derive(Serialize)]
    struct AnimOutput { objects: Vec<Object>, offsets: [u8; 3] }
    let cases: Vec<_> = every_offset.iter().step_by(3).map(|&offsets| {
        let (objects, after) = anim_wheels(&mut oracle, offsets);
        Case { input: offsets, output: AnimOutput { objects, offsets: after }, rng: vec![] }
    }).collect();
    write_fixture("slots", "anim_wheels", &cases);

    #[derive(Serialize)]
    struct MatchCase { offsets: [u8; 3], bet: u8, flags: u8, reroll: u8 }
    let mut cases = Vec::new();
    for &offsets in &every_offset {
        for bet in 1..=3u8 {
            let flags = [0, CAN_WIN, CAN_WIN_WITH_7_OR_BAR, CAN_WIN | CAN_WIN_WITH_7_OR_BAR][rng.random_range(0..4)];
            let reroll = rng.random_range(1..=4);
            let output = check_for_matches(&mut oracle, offsets, bet, flags, reroll);
            cases.push(Case { input: MatchCase { offsets, bet, flags, reroll }, output, rng: vec![] });
        }
    }
    let won = cases.iter().filter(|case| matches!(case.output.0, Matches::Won(_))).count();
    assert!(won > 20, "only {won} wins among {}", cases.len());
    write_fixture("slots", "check_for_matches", &cases);

    #[derive(Serialize)]
    struct RewardCase { symbol: u8, flags: u8, allow_matches_counter: u8 }
    let mut cases = Vec::new();
    for symbol in [SEVEN, BAR, CHERRY, FISH, BIRD, MOUSE] {
        for flags in [0, CAN_WIN, CAN_WIN_WITH_7_OR_BAR, CAN_WIN | CAN_WIN_WITH_7_OR_BAR] {
            for counter in [0, 1, 2, 60] {
                for byte in (0..=255u8).step_by(8) {
                    seed(&mut oracle, byte);
                    let input = RewardCase { symbol, flags, allow_matches_counter: counter };
                    let (output, rng) = slot_reward(&mut oracle, symbol, flags, counter);
                    cases.push(Case { input, output, rng });
                }
            }
        }
    }
    write_fixture("slots", "slot_reward", &cases);

    let cases: Vec<_> = (0..=255u8).map(|byte| {
        seed(&mut oracle, byte);
        let (output, rng) = lucky_slot_machine(&mut oracle);
        Case { input: (), output, rng }
    }).collect();
    let lucky: std::collections::BTreeSet<u8> = cases.iter().map(|case| case.output).collect();
    assert!(lucky.len() > 20, "only {} machines were ever picked", lucky.len());
    write_fixture("slots", "lucky_slot_machine", &cases);
}
