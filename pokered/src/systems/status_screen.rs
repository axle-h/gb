//! What the status screen computes and draws without waiting on a frame: `CalcExpToLevelUp`,
//! `PrintStatsBox`, `DrawLineBox`, `PrintMonType`, `PrintLevel` and `PrintStatusCondition`.
//!
//! Exact: the experience still to go is 24 bits and wraps, as the three `sbc`s leave it; the types
//! printed are the *species'* from its base stats rather than the mon's own two bytes; a species
//! with one type has its `TYPE2/` label rubbed out rather than a second name printed.

use poke_core::base_stats::BaseStats;
use poke_core::rom_gfx::rom_slice;
use poke_core::species::PokemonSpecies;
use poke_core::symbols::{pokered_symbols, DmgPointer};
use crate::gfx::ui::{UiSurface, SCREEN_TILES_X};
use crate::modes::place_string::ligature;
use crate::party::NUM_STATS;
use crate::systems::experience::calc_experience;
use crate::systems::print_num::{print_number, NumberFormat};

pub const MAX_LEVEL: u8 = 100;
const EXPERIENCE_MASK: u32 = 0xFF_FFFF;

/// `<LV>`, `│`, `┘`, `─` and the half arrow a line box ends in.
pub const LV: u8 = 0x6E;
const VERTICAL: u8 = 0x78;
const CORNER: u8 = 0x77;
const HORIZONTAL: u8 = 0x76;
const HALF_ARROW: u8 = 0x6F;
const NEXT: u8 = 0x4E;
const TERMINATOR: u8 = 0x50;

/// `CalcExpToLevelUp`: what the next level needs less what the mon has, or 0 at level 100. The
/// level is incremented as a byte, so 255 asks about level 0.
pub fn calc_exp_to_level_up(growth_rate: u8, level: u8, exp: u32) -> u32 {
    if level == MAX_LEVEL {
        return 0;
    }
    calc_experience(growth_rate, level.wrapping_add(1)).wrapping_sub(exp) & EXPERIENCE_MASK
}

fn put(ui: &mut UiSurface, at: usize, tile: u8) {
    ui.set(at % SCREEN_TILES_X, at / SCREEN_TILES_X, tile);
}

/// `PlaceString` with nothing to delay it, for the screen's fixed strings: `<NEXT>` goes back to
/// the column the string started in, one row down when `single_spaced` and two otherwise, and
/// `<POKE>` and its kind are expanded. Stops at `@` or the end.
pub fn place_lines(ui: &mut UiSurface, at: usize, bytes: &[u8], single_spaced: bool) {
    let (mut line, mut cursor) = (at, at);
    for &byte in bytes {
        match byte {
            TERMINATOR => return,
            NEXT => {
                line += if single_spaced { 1 } else { 2 } * SCREEN_TILES_X;
                cursor = line;
            }
            letter => {
                for &tile in ligature(letter).unwrap_or(std::slice::from_ref(&letter)) {
                    put(ui, cursor, tile);
                    cursor += 1;
                }
            }
        }
    }
}

/// `PrintLevel`: `<LV>` and two digits, or from level 100 three digits written over the `<LV>`.
pub fn print_level(ui: &mut UiSurface, at: usize, level: u8) {
    put(ui, at, LV);
    let (at, digits) = if level >= MAX_LEVEL { (at, 3) } else { (at + 1, 2) };
    print_number(ui, at, level as u32, NumberFormat { digits, left_align: true, leading_zeroes: false });
}

/// `PrintStatusCondition`: `FNT` for a mon with no HP whatever its status byte says, then the
/// first ailment in `PrintStatusAilment`'s order. False when nothing was printed, which is the
/// cartridge's zero flag.
pub fn print_status_condition(ui: &mut UiSurface, at: usize, status: u8, hp: u16) -> bool {
    let text = match status {
        _ if hp == 0 => "FNT",
        s if s & 1 << 3 != 0 => "PSN",
        s if s & 1 << 4 != 0 => "BRN",
        s if s & 1 << 5 != 0 => "FRZ",
        s if s & 1 << 6 != 0 => "PAR",
        s if s & 0b111 != 0 => "SLP",
        _ => return false,
    };
    place_lines(ui, at, &encode(text), false);
    true
}

/// `DrawLineBox`: a line `height` tiles down from `at`, a corner, then `width` tiles back to the
/// left and the half arrow that ends it.
pub fn draw_line_box(ui: &mut UiSurface, at: usize, height: usize, width: usize) {
    let mut cursor = at;
    for _ in 0..height {
        put(ui, cursor, VERTICAL);
        cursor += SCREEN_TILES_X;
    }
    put(ui, cursor, CORNER);
    for _ in 0..width {
        cursor -= 1;
        put(ui, cursor, HORIZONTAL);
    }
    put(ui, cursor - 1, HALF_ARROW);
}

/// `TypeNames`, indexed by the type byte: the unused ids between `GHOST` and `FIRE` point at
/// `NORMAL`.
pub fn type_name(id: u8) -> Vec<u8> {
    let table = rom_slice(pokered_symbols::TypeNames);
    let address = u16::from_le_bytes([table[id as usize * 2], table[id as usize * 2 + 1]]);
    let name = rom_slice(DmgPointer { bank: pokered_symbols::TypeNames.bank, address });
    name.iter().copied().take_while(|&byte| byte != TERMINATOR).collect()
}

/// `PrintMonType` at `at`: the species' first type, then its second two rows down, or with only
/// one the six tiles of `TYPE2/` on the row between blanked.
pub fn print_mon_type(ui: &mut UiSurface, at: usize, species: PokemonSpecies) {
    let [first, second] = BaseStats::of(species).types;
    place_lines(ui, at, &type_name(first), false);
    if first == second {
        let from = at + SCREEN_TILES_X - 1;
        for i in 0..6 {
            put(ui, from + i, UiSurface::BLANK);
        }
    } else {
        place_lines(ui, at + 2 * SCREEN_TILES_X, &type_name(second), false);
    }
}

/// `PrintStatsBox`'s `d`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatsBox {
    /// `STATUS_SCREEN_STATS_BOX`, at the lower left.
    StatusScreen,
    /// `LEVEL_UP_STATS_BOX`, at the upper right, which a battle and a Rare Candy show.
    LevelUp,
}

/// `PrintStatsBox`: a box, the four stat names down it, and each value three digits wide on the
/// row under its name. `stats` is max HP first, as a party mon keeps them; HP is not printed.
pub fn print_stats_box(ui: &mut UiSurface, kind: StatsBox, stats: [u16; NUM_STATS]) {
    let (corner, text, offset) = match kind {
        StatsBox::StatusScreen => ((0, 8, 8), (1, 9), SCREEN_TILES_X + 5),
        StatsBox::LevelUp => ((9, 2, 9), (11, 3), SCREEN_TILES_X + 4),
    };
    ui.text_box_border(corner.0, corner.1, corner.2, 8);
    let at = text.1 * SCREEN_TILES_X + text.0;
    place_lines(ui, at, &encode("ATTACK<NEXT>DEFENSE<NEXT>SPEED<NEXT>SPECIAL"), false);
    let format = NumberFormat { digits: 3, left_align: false, leading_zeroes: false };
    for (row, &stat) in stats[1..].iter().enumerate() {
        print_number(ui, at + offset + row * 2 * SCREEN_TILES_X, stat as u32, format);
    }
}

pub fn encode(text: &str) -> Vec<u8> {
    poke_core::charmap::encode(text).expect("the status screen's words encode")
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;
    use crate::fixtures::cases;
    use super::*;

    #[derive(Deserialize)]
    struct ExpInput {
        growth_rate: u8,
        level: u8,
        exp: u32,
    }

    #[test]
    fn every_harvested_case_of_calc_exp_to_level_up() {
        for (i, output, _) in cases::<ExpInput, u32>(include_str!("../../fixtures/status_screen/calc_exp_to_level_up.jsonl")) {
            assert_eq!(calc_exp_to_level_up(i.growth_rate, i.level, i.exp), output,
                "rate {} level {} exp {}", i.growth_rate, i.level, i.exp);
        }
    }

    #[derive(Deserialize)]
    struct TypeInput {
        species: PokemonSpecies,
    }

    /// The four rows `PrintMonType` can touch from (11, 10), as the cartridge left them over a
    /// template with `TYPE2/` already on the row between.
    #[test]
    fn every_harvested_case_of_print_mon_type() {
        for (i, rows, _) in cases::<TypeInput, Vec<Vec<u8>>>(include_str!("../../fixtures/status_screen/print_mon_type.jsonl")) {
            let mut ui = UiSurface::default();
            place_lines(&mut ui, 9 * SCREEN_TILES_X + 10, &encode("TYPE1/<NEXT>TYPE2/"), false);
            print_mon_type(&mut ui, 10 * SCREEN_TILES_X + 11, i.species);
            let drawn: Vec<Vec<u8>> = (9..13).map(|y| ui.row(y).to_vec()).collect();
            assert_eq!(drawn, rows, "{}", i.species);
        }
    }

    #[test]
    fn level_100_needs_nothing_more() {
        assert_eq!(calc_exp_to_level_up(0, 100, 1_000_000), 0);
        assert_eq!(calc_exp_to_level_up(0, 99, 970_299), 1_000_000 - 970_299);
    }

    #[test]
    fn a_line_box_ends_in_a_half_arrow() {
        let mut ui = UiSurface::default();
        draw_line_box(&mut ui, SCREEN_TILES_X + 19, 6, 10);
        assert_eq!(ui.get(19, 1), VERTICAL);
        assert_eq!(ui.get(19, 7), CORNER);
        assert_eq!(ui.row(7)[9..19], [HORIZONTAL; 10]);
        assert_eq!(ui.get(8, 7), HALF_ARROW);
    }

    #[test]
    fn a_level_from_100_covers_its_own_lv() {
        let mut ui = UiSurface::default();
        print_level(&mut ui, 0, 7);
        assert_eq!(ui.row(0)[..2], [LV, 0xF6 + 7]);
        print_level(&mut ui, SCREEN_TILES_X, 100);
        assert_eq!(ui.row(1)[..3], encode("100")[..]);
    }
}
