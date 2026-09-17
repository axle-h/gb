//! The Game Corner slot machine's arithmetic: `SlotMachineWheel1`..`3`, `SlotMachine_SetFlags`,
//! `SlotMachine_StopOrAnimWheel1`..`3`, `SlotMachine_CheckForMatches`, `SlotReward*Func` and
//! `GameCornerSelectLuckySlotMachine`. The machine decides whether the player may win before the
//! wheels move, and the wheels are then stopped where that answer needs them.

use poke_core::rom_gfx::{rom_slice, TILE_BYTES};
use poke_core::symbols::pokered_symbols as sym;
use serde::{Deserialize, Serialize};
use crate::gfx::layers::Object;
use crate::gfx::ui::SCREEN_TILES_X;
use crate::rng::Rng;

/// The symbols as the cartridge compares them: `HIGH(SLOTS7)` and its neighbours. A symbol is four
/// tiles, `n`..`n + 3`, and a wheel table holds each as a `dw` whose low byte is the top pair of
/// tiles and whose high byte is the bottom pair; a comparison only ever sees the high byte.
pub const SEVEN: u8 = 0x02;
pub const BAR: u8 = 0x06;
pub const CHERRY: u8 = 0x0A;
pub const FISH: u8 = 0x0E;
pub const BIRD: u8 = 0x12;
pub const MOUSE: u8 = 0x16;

/// `wSlotMachineFlags`.
pub const CAN_WIN: u8 = 1 << 6;
pub const CAN_WIN_WITH_7_OR_BAR: u8 = 1 << 7;

/// `wSlotMachineWheel1Offset`, in half symbols: `SlotMachine_AnimWheel` advances it one and wraps
/// it here, so a wheel has fifteen resting places rather than its table's eighteen.
pub const WHEEL_WRAP: u8 = 30;

/// `wSlotMachineWheel1SlipCounter`, `wSlotMachineWheel2SlipCounter` and `wSlotMachineRerollCounter`,
/// all three set from the same `ld a, 4` each bet.
pub const SLIP: u8 = 4;

/// `wSlotMachineSevenAndBarModeChance`: a random byte above this allows three sevens or three bars.
/// `GameCornerSelectLuckySlotMachine` picks one machine a visit to run at [`LUCKY`].
pub const NOT_LUCKY: u8 = 253;
pub const LUCKY: u8 = 250;

/// The tile a lit ball is drawn from, and the one `SlotMachine_PutOutLitBalls` puts back.
pub const BALL_LIT: u8 = 0x14;
pub const BALL_OUT: u8 = 0x23;

/// A wheel table, 36 bytes. A wheel is read a byte at a time, so a half symbol at a time.
pub fn wheel(index: usize) -> &'static [u8] {
    let table = [sym::SlotMachineWheel1, sym::SlotMachineWheel2, sym::SlotMachineWheel3][index];
    &rom_slice(table)[..36]
}

/// `SlotMachine_GetWheel1Tiles`..`3`: the bottom, middle and top symbol of each wheel, read at a
/// stride of two from the wheel's offset. At an even offset these are the top halves of three
/// symbols, which is a wheel mid-spin; a wheel only stops on an odd one.
pub fn wheel_tiles(offsets: [u8; 3]) -> [[u8; 3]; 3] {
    std::array::from_fn(|w| std::array::from_fn(|i| wheel(w)[offsets[w] as usize + 2 * i]))
}

/// `SlotMachine_SetFlags`, once a bet is placed. It reads `Random` only when it has a choice to
/// make: seven-and-bar mode, once entered, lasts until a reward clears it, and a non-zero
/// `wSlotMachineAllowMatchesCounter` allows a match without asking.
pub fn set_flags(rng: &mut impl Rng, flags: &mut u8, allow_matches_counter: &mut u8, chance: u8) {
    if *flags & CAN_WIN_WITH_7_OR_BAR != 0 {
        return;
    }
    if *allow_matches_counter != 0 {
        *flags |= CAN_WIN;
        return;
    }
    let roll = rng.random();
    if roll == 0 {
        *allow_matches_counter = 60;
    } else if chance < roll {
        *flags |= CAN_WIN_WITH_7_OR_BAR;
    } else if roll > 210 {
        *flags |= CAN_WIN;
    } else {
        *flags = 0;
    }
}

/// `GameCornerSelectLuckySlotMachine`, run when the Game Corner loads: which machine of the twelve
/// pays better this visit, as a `wHiddenEventIndex` counting from one. A roll under seven is read
/// as eight, so the answer is one rather than zero.
pub fn lucky_slot_machine(rng: &mut impl Rng) -> u8 {
    let roll = rng.random();
    if roll < 7 { 8 >> 3 } else { roll >> 3 }
}

/// `wSlotMachineSevenAndBarModeChance` for the machine this hidden event stands at.
pub fn seven_and_bar_mode_chance(lucky: u8, hidden_event_index: u8) -> u8 {
    if hidden_event_index.wrapping_add(1) == lucky { LUCKY } else { NOT_LUCKY }
}

/// Where the three wheels stand and how much slip each of the first two has left.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Wheels {
    /// `wSlotMachineWheel1Offset`..`3`.
    pub offsets: [u8; 3],
    /// `wSlotMachineWheel1SlipCounter` and wheel 2's: how many more stoppable frames each may
    /// refuse to stop on.
    pub slip: [u8; 2],
}

impl Wheels {
    /// `LoadSlotMachineTiles` leaves all three offsets at `$1c`.
    pub const START: u8 = 0x1C;

    pub fn new() -> Self {
        Self { offsets: [Self::START; 3], slip: [0; 2] }
    }

    /// The slip a bet buys each of the first two wheels.
    pub fn bet(&mut self) {
        self.slip = [SLIP; 2];
    }

    /// `SlotMachine_AnimWheel`: one half symbol on, wrapping at [`WHEEL_WRAP`].
    pub fn anim(&mut self, index: usize) {
        self.offsets[index] = (self.offsets[index] + 1) % WHEEL_WRAP;
    }

    /// `SlotMachine_StopOrAnimWheel1`..`3` in turn, `stopping` being
    /// `wStoppingWhichSlotMachineWheel`. True when wheel 3 has come to rest, which ends the spin.
    pub fn stop_or_anim(&mut self, stopping: u8, flags: u8) -> bool {
        for index in 0..2 {
            if !self.stops(index, stopping, flags) {
                self.anim(index);
            }
        }
        // Wheel 3 stops as soon as a symbol is centred.
        if stopping >= 3 && self.offsets[2] % 2 == 1 {
            return true;
        }
        self.anim(2);
        false
    }

    /// Whether wheel 1 or 2 stays where it is this frame. A wheel is asked only once its turn has
    /// come and only on an odd offset, where a symbol is centred rather than two halves showing.
    fn stops(&mut self, index: usize, stopping: u8, flags: u8) -> bool {
        if stopping < index as u8 + 1 || self.offsets[index] % 2 == 0 {
            return false;
        }
        if self.slip[index] == 0 {
            return true;
        }
        self.slip[index] -= 1;
        let stop = if index == 0 { self.stop_wheel1_early(flags) } else { self.stop_wheel2_early(flags) };
        if stop {
            self.slip[index] = 0;
        }
        stop
    }

    /// `SlotMachine_StopWheel1Early`: wheel 1 stops on anything but a cherry. In seven-and-bar mode
    /// its loop compares each tile with `HIGH(SLOTS7)` for *less than*, which no symbol ever is, so
    /// the wheel never stops early and lands wherever the slip counter runs out.
    fn stop_wheel1_early(&self, flags: u8) -> bool {
        flags & CAN_WIN_WITH_7_OR_BAR == 0 && wheel_tiles(self.offsets)[0][1] != CHERRY
    }

    /// `SlotMachine_StopWheel2Early`: wheel 2 stops where wheels 1 and 2 could still line up. In
    /// seven-and-bar mode it stops on a seven or a bar instead, read off the row the match search
    /// left pointing at, which with no match at all is wheel 2's bottom symbol.
    fn stop_wheel2_early(&self, flags: u8) -> bool {
        let tiles = wheel_tiles(self.offsets);
        let matched = find_wheel1_wheel2_matches(&tiles);
        if flags & CAN_WIN_WITH_7_OR_BAR == 0 {
            return matched.is_some();
        }
        tiles[1][matched.unwrap_or(0)] <= BAR
    }
}

/// `SlotMachine_FindWheel1Wheel2Matches`: which of wheel 2's three symbols could still make a line
/// with wheel 1, or `None`. The order is the cartridge's, and the answer is the row it left `de`
/// on, which seven-and-bar mode reads even when there is no match.
fn find_wheel1_wheel2_matches(tiles: &[[u8; 3]; 3]) -> Option<usize> {
    let (one, two) = (tiles[0], tiles[1]);
    if two[0] == one[0] {
        return Some(0);
    }
    if two[1] == one[0] || two[1] == one[1] || two[1] == one[2] {
        return Some(1);
    }
    (two[2] == one[2]).then_some(2)
}

/// What one pass of `SlotMachine_CheckForMatches` decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Matches {
    /// `.acceptMatch`: the symbol lined up, as a tile value.
    Won(u8),
    /// `.rollWheel3DownByOneSymbol`: wheel 3 moves a symbol on and the search runs again.
    Reroll,
    /// `.noMatch`.
    Lost,
}

/// The lines a bet buys, as a row of each wheel with 0 the bottom: one coin buys the middle row,
/// two the top and bottom as well, three the two diagonals on top of those. The order is the one
/// the cartridge searches in, since the first line found is the one paid.
fn lines(bet: u8) -> &'static [(usize, usize, usize)] {
    const LINES: [(usize, usize, usize); 5] = [(0, 1, 2), (2, 1, 0), (2, 2, 2), (0, 0, 0), (1, 1, 1)];
    match bet {
        3 => &LINES[..],
        2 => &LINES[2..],
        _ => &LINES[4..],
    }
}

/// `SlotMachine_CheckForMatches`, one pass over the wheels as they stand. A match the flags do not
/// allow is rolled away rather than paid, and so is no match at all while the reroll counter lasts.
/// A roll the flags forbid does not spend the counter, so only the wheels themselves end it.
pub fn check_for_matches(offsets: [u8; 3], bet: u8, flags: u8, reroll: &mut u8) -> Matches {
    let tiles = wheel_tiles(offsets);
    let found = lines(bet).iter()
        .find(|&&(a, b, c)| tiles[0][a] == tiles[1][b] && tiles[0][a] == tiles[2][c])
        .map(|&(a, _, _)| tiles[0][a]);
    let allowed = flags & (CAN_WIN | CAN_WIN_WITH_7_OR_BAR);
    match found {
        Some(_) if allowed == 0 => Matches::Reroll,
        Some(symbol) if flags & CAN_WIN_WITH_7_OR_BAR != 0 => Matches::Won(symbol),
        Some(symbol) if symbol <= BAR => Matches::Reroll,
        Some(symbol) => Matches::Won(symbol),
        None if allowed == 0 => Matches::Lost,
        None => {
            *reroll = reroll.wrapping_sub(1);
            if *reroll == 0 { Matches::Lost } else { Matches::Reroll }
        }
    }
}

/// What a lined-up symbol pays and how long the screen flashes for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reward {
    /// `wPayoutCoins`.
    pub coins: u16,
    /// `b` at `.flashScreenLoop`, which runs that many times five frames apart.
    pub flashes: u8,
}

/// `SlotRewardPointers`' function for the symbol: the payout, and what winning it costs the
/// machine. Three sevens end seven-and-bar mode more often than not and always spend the whole of
/// the counter; three bars end it outright; the smaller wins spend one of the counter.
pub fn slot_reward(rng: &mut impl Rng, symbol: u8, flags: &mut u8, allow_matches_counter: &mut u8) -> Reward {
    match symbol {
        SEVEN => {
            if rng.random() >= 0x80 {
                *flags = 0;
            }
            *allow_matches_counter = 0;
            Reward { coins: 300, flashes: 0x14 }
        }
        BAR => {
            *flags = 0;
            Reward { coins: 100, flashes: 8 }
        }
        CHERRY | FISH | BIRD | MOUSE => {
            *allow_matches_counter = allow_matches_counter.saturating_sub(1);
            if symbol == CHERRY { Reward { coins: 8, flashes: 2 } } else { Reward { coins: 15, flashes: 4 } }
        }
        _ => unreachable!("a line is one of the six symbols"),
    }
}

/// `SlotReward*Text`, four bytes of it as the cartridge copies into `wStringBuffer`. The shorter
/// ones bring two bytes of the next text with them, which the `@` between them never lets show.
pub fn reward_text(symbol: u8) -> Vec<u8> {
    let at = match symbol {
        SEVEN => sym::SlotReward300Text,
        BAR => sym::SlotReward100Text,
        CHERRY => sym::SlotReward8Text,
        _ => sym::SlotReward15Text,
    };
    rom_slice(at)[..4].to_vec()
}

/// `SlotMachine_AnimWheel`: the twelve objects a wheel is drawn from, six rows of two, bottom row
/// first, at `wShadowOAMSprite00`, `12` and `24`. Each row is a half symbol.
pub fn anim_wheel_objects(index: usize, offset: u8) -> Vec<Object> {
    const BASE_X: [u8; 3] = [0x30, 0x50, 0x70];
    const BOTTOM_Y: u8 = 0x58;
    let table = wheel(index);
    (0..6).flat_map(|row| {
        let (y, tile) = (BOTTOM_Y - 8 * row as u8, table[offset as usize + row]);
        [
            Object { y, x: BASE_X[index], tile, attributes: Object::BEHIND_BG },
            Object { y, x: BASE_X[index] + 8, tile: tile + 1, attributes: Object::BEHIND_BG },
        ]
    }).collect()
}

/// The rows `SlotMachine_LightBalls` reaches for a bet: one coin lights the middle pair of balls,
/// two the pair either side of it as well, three the outermost pair on top of those. Putting them
/// out reaches all five.
pub fn lit_ball_rows(bet: u8) -> &'static [usize] {
    const ROWS: [usize; 5] = [2, 10, 4, 8, 6];
    match bet {
        3 => &ROWS[..],
        2 => &ROWS[2..],
        _ => &ROWS[4..],
    }
}

/// `SlotMachine_UpdateBallTiles`: a ball is two tiles tall at column 3 and column 16 of `row`, and
/// the lower half is the tile after the upper.
pub fn ball_tiles(row: usize, tile: u8) -> [(usize, u8); 4] {
    let at = row * SCREEN_TILES_X + 3;
    [(at, tile), (at + 13, tile), (at + 20, tile + 1), (at + 33, tile + 1)]
}

/// `SlotMachineTiles2`, the symbols. The cartridge copies `$1c` tiles where the data is only `$18`
/// long, so four tiles of whatever follows it come along, both times it is loaded.
pub fn symbol_tiles() -> &'static [u8] {
    &rom_slice(sym::SlotMachineTiles2)[..0x1C * TILE_BYTES]
}

/// `SlotMachineTiles1`, the cabinet.
pub fn cabinet_tiles() -> &'static [u8] {
    let len = (sym::SlotMachineTiles1End.address - sym::SlotMachineTiles1.address) as usize;
    &rom_slice(sym::SlotMachineTiles1)[..len]
}

/// `SlotMachineMap`, the cabinet as a screen: twenty by eighteen tiles.
pub fn screen_map() -> &'static [u8] {
    let len = (sym::SlotMachineMapEnd.address - sym::SlotMachineMap.address) as usize;
    &rom_slice(sym::SlotMachineMap)[..len]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::cases;
    use crate::rng::GameRng;

    #[derive(serde::Deserialize)]
    struct FlagsCase {
        flags: u8,
        allow_matches_counter: u8,
        chance: u8,
    }

    #[test]
    fn every_harvested_case_of_set_flags() {
        for (input, output, rng) in cases::<FlagsCase, (u8, u8)>(include_str!("../../fixtures/slots/set_flags.jsonl")) {
            let (mut flags, mut counter) = (input.flags, input.allow_matches_counter);
            set_flags(&mut GameRng::tape(rng), &mut flags, &mut counter, input.chance);
            assert_eq!((flags, counter), output,
                "flags ${:02X}, counter {}, chance {}", input.flags, input.allow_matches_counter, input.chance);
        }
    }

    #[test]
    fn every_harvested_case_of_wheel_tiles() {
        for (offsets, tiles, _) in cases::<[u8; 3], [[u8; 3]; 3]>(include_str!("../../fixtures/slots/wheel_tiles.jsonl")) {
            assert_eq!(wheel_tiles(offsets), tiles, "{offsets:?}");
        }
    }

    #[derive(serde::Deserialize)]
    struct SpinCase {
        wheels: Wheels,
        stopping: u8,
        flags: u8,
    }

    #[derive(Debug, PartialEq, Eq, serde::Deserialize)]
    struct SpinOutput {
        wheels: Wheels,
        stopped: bool,
    }

    #[test]
    fn every_harvested_case_of_a_spinning_frame() {
        for (input, output, _) in cases::<SpinCase, SpinOutput>(include_str!("../../fixtures/slots/stop_or_anim_wheels.jsonl")) {
            let mut wheels = input.wheels;
            let stopped = wheels.stop_or_anim(input.stopping, input.flags);
            assert_eq!(SpinOutput { wheels, stopped }, output,
                "{:?} stopping {} flags ${:02X}", input.wheels, input.stopping, input.flags);
        }
    }

    #[derive(Debug, PartialEq, Eq, serde::Deserialize)]
    struct AnimOutput {
        objects: Vec<Object>,
        offsets: [u8; 3],
    }

    #[test]
    fn every_harvested_case_of_the_objects_a_wheel_is_drawn_from() {
        for (offsets, output, _) in cases::<[u8; 3], AnimOutput>(include_str!("../../fixtures/slots/anim_wheels.jsonl")) {
            let objects = (0..3).flat_map(|w| anim_wheel_objects(w, offsets[w])).collect();
            let mut wheels = Wheels { offsets, slip: [0; 2] };
            (0..3).for_each(|w| wheels.anim(w));
            assert_eq!(AnimOutput { objects, offsets: wheels.offsets }, output, "{offsets:?}");
        }
    }

    #[derive(serde::Deserialize)]
    struct MatchCase {
        offsets: [u8; 3],
        bet: u8,
        flags: u8,
        reroll: u8,
    }

    #[test]
    fn every_harvested_case_of_check_for_matches() {
        for (input, output, _) in cases::<MatchCase, (Matches, u8)>(include_str!("../../fixtures/slots/check_for_matches.jsonl")) {
            let mut reroll = input.reroll;
            let found = check_for_matches(input.offsets, input.bet, input.flags, &mut reroll);
            assert_eq!((found, reroll), output,
                "{:?} bet {} flags ${:02X} reroll {}", input.offsets, input.bet, input.flags, input.reroll);
        }
    }

    #[derive(serde::Deserialize)]
    struct RewardCase {
        symbol: u8,
        flags: u8,
        allow_matches_counter: u8,
    }

    #[test]
    fn every_harvested_case_of_a_reward() {
        for (input, output, rng) in cases::<RewardCase, (Reward, u8, u8)>(include_str!("../../fixtures/slots/slot_reward.jsonl")) {
            let (mut flags, mut counter) = (input.flags, input.allow_matches_counter);
            let reward = slot_reward(&mut GameRng::tape(rng), input.symbol, &mut flags, &mut counter);
            assert_eq!((reward, flags, counter), output, "symbol ${:02X}", input.symbol);
        }
    }

    #[test]
    fn every_harvested_case_of_the_lucky_machine() {
        for (_, lucky, rng) in cases::<(), u8>(include_str!("../../fixtures/slots/lucky_slot_machine.jsonl")) {
            assert_eq!(lucky_slot_machine(&mut GameRng::tape(rng.clone())), lucky, "{rng:?}");
        }
    }

    /// A wheel is eighteen symbols but fifteen resting places, so its last row is never the bottom
    /// symbol and its very last byte is never drawn at all.
    #[test]
    fn a_wheel_rests_on_fifteen_of_its_eighteen_rows() {
        let bottoms: Vec<u8> = (0..WHEEL_WRAP).filter(|o| o % 2 == 1).map(|o| wheel(0)[o as usize]).collect();
        assert_eq!(bottoms, (0..15).map(|i| wheel(0)[2 * i + 1]).collect::<Vec<_>>());
        assert!(wheel(0).len() > 30, "three rows past the last resting place");
    }
}
