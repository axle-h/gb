//! `engine/gfx/hp_bar.asm` and `DrawHPBar`: how long the bar is, what colour it is, and the tiles
//! it is drawn from.
//!
//! The colour is not only for the SGB. `wPartyMenuHPBarColors` is what `AnimatePartyMon` indexes to
//! pick its speed, so a party mon on red HP animates six times slower than one on green.

use serde::{Deserialize, Serialize};
use crate::gfx::ui::{UiSurface, SCREEN_TILES_X};
use crate::systems::math::{divide, multiply};
use crate::systems::print_num::{print_number, NumberFormat};

/// The bar is six tiles of eight pixels.
pub const BAR_TILES: u8 = 6;
const BAR_PIXELS: u8 = 48;

/// `wHPBarType`, which only changes the tile the bar ends with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HpBarType {
    StatusScreenOrBattle,
    PartyMenu,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HpBarColour {
    Green,
    Yellow,
    Red,
}

impl HpBarColour {
    /// `PartyMonSpeeds`: the V-blanks one animation frame lasts, before the SGB adds its own.
    pub fn animation_speed(self) -> u8 {
        match self {
            Self::Green => 5,
            Self::Yellow => 16,
            Self::Red => 32,
        }
    }
}

/// `GetHPBarLength`'s arguments: `bc` is the current HP and `de` the maximum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HpBarInput {
    pub current: u16,
    pub max: u16,
}

/// `GetHPBarLength`: `current * 48 / max`, floored, and never less than one pixel.
///
/// A max over 255 is divided by four, and so is the product, because the divisor is one byte. Both
/// shifts keep only the low sixteen bits of the product, which is where `hMultiplicand` sits: it
/// overlaps `hProduct` from its second byte.
pub fn hp_bar_length(current: u16, max: u16) -> u8 {
    let mut product = multiply(current as u32, BAR_PIXELS).to_be_bytes();
    let divisor = if max >= 256 {
        let low = u16::from_be_bytes([product[2], product[3]]) >> 2;
        [product[2], product[3]] = low.to_be_bytes();
        (max >> 2) as u8
    } else {
        max as u8
    };
    let (quotient, _) = divide(product, divisor, 4);
    quotient[3].max(1)
}

/// `GetHealthBarColor`, in pixels of the 48.
pub fn health_bar_colour(pixels: u8) -> HpBarColour {
    match pixels {
        27.. => HpBarColour::Green,
        10..=26 => HpBarColour::Yellow,
        _ => HpBarColour::Red,
    }
}

/// `DrawHPBar`: `HP:` and then `tiles` tiles filled to `pixels`. `sliver` is the cartridge's `c`,
/// which shows one lit pixel for a mon that is alive but rounds to nothing.
pub fn draw_hp_bar(ui: &mut UiSurface, at: usize, tiles: u8, mut pixels: u8, sliver: bool, kind: HpBarType) {
    let put = |ui: &mut UiSurface, at: usize, tile: u8| ui.set(at % SCREEN_TILES_X, at / SCREEN_TILES_X, tile);
    put(ui, at, 0x71);
    put(ui, at + 1, 0x62);
    let bar = at + 2;
    for tile in 0..tiles as usize {
        put(ui, bar + tile, 0x63);
    }
    put(ui, bar + tiles as usize, match kind {
        HpBarType::StatusScreenOrBattle => 0x6D,
        HpBarType::PartyMenu => 0x6C,
    });
    if pixels == 0 {
        if !sliver {
            return;
        }
        pixels = 1;
    }
    let mut tile = bar;
    while pixels >= 8 {
        put(ui, tile, 0x6B);
        tile += 1;
        pixels -= 8;
        if pixels == 0 {
            return;
        }
    }
    put(ui, tile, 0x63 + pixels);
}

/// `DrawHP2`: the bar with the fraction beside it, as the party menu draws it. `beside` is
/// `BIT_PARTY_MENU_HP_BAR`; without it the fraction goes under the bar instead.
pub fn draw_hp(ui: &mut UiSurface, at: usize, current: u16, max: u16, beside: bool, kind: HpBarType) -> HpBarColour {
    let pixels = if current == 0 { 0 } else { hp_bar_length(current, max) };
    draw_hp_bar(ui, at, BAR_TILES, pixels, current != 0, kind);
    let fraction = if beside { at + 9 } else { at + SCREEN_TILES_X + 1 };
    let format = NumberFormat { digits: 3, leading_zeroes: false, left_align: false };
    let end = print_number(ui, fraction, current as u32, format);
    ui.set(end % SCREEN_TILES_X, end / SCREEN_TILES_X, 0xF3);
    print_number(ui, end + 1, max as u32, format);
    health_bar_colour(pixels)
}

/// `UpdateHPBar2` with `BIT_PARTY_MENU_HP_BAR` set: the bar and the number beside it walk from the
/// old HP to the new a point at a time.
///
/// Each point costs the frame `UpdateHPBar_PrintHPNumber` waits after printing, and two frames for
/// every pixel the bar moves; the end costs one frame of printing and two of drawing. The `Delay3`
/// the routine ends with is loading and not modelled.
/// Both the number and the bar lag a point behind: each pass prints the HP it is leaving and draws
/// from the pixel it is leaving. A mon at 0 HP counts as one pixel, as `GetHPBarLength` has it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HpBarAnimation {
    /// `hl`: the `HP:` tile.
    at: usize,
    max: u16,
    /// `wHPBarOldHP`, and the point the pass under way is moving to.
    old: u16,
    next: u16,
    target: u16,
    kind: HpBarType,
    /// `e`, the pixels drawn, and the ticks `UpdateHPBar_AnimateHPBar` has left.
    pixels: u8,
    ticks: u8,
    wait: u8,
    stage: Stage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Pass,
    Tick,
    AfterTick,
    Last,
    Done,
}

impl HpBarAnimation {
    /// `None` where the HP does not change, which `UpdateHPBar2` returns on at once.
    pub fn new(at: usize, max: u16, old: u16, new: u16, kind: HpBarType) -> Option<Self> {
        (old != new).then_some(Self {
            at, max, old, next: old, target: new, kind, pixels: 0, ticks: 0, wait: 0, stage: Stage::Pass,
        })
    }

    /// One frame; true in the frame the routine returns.
    pub fn update(&mut self, ui: &mut UiSurface) -> bool {
        if self.wait > 0 {
            self.wait -= 1;
            if self.wait > 0 {
                return false;
            }
        }
        loop {
            match self.stage {
                Stage::Pass if self.old == self.target => {
                    self.pixels = if self.target == 0 { 0 } else { hp_bar_length(self.target, self.max) };
                    self.print(ui);
                    self.stage = Stage::Last;
                    self.wait = 1;
                    return false;
                }
                Stage::Pass => {
                    self.next = if self.target > self.old { self.old + 1 } else { self.old - 1 };
                    let (from, to) = (hp_bar_length(self.old, self.max), hp_bar_length(self.next, self.max));
                    self.print(ui);
                    self.wait = 1;
                    self.pixels = from;
                    self.ticks = from.abs_diff(to);
                    if self.ticks == 0 {
                        self.old = self.next;
                    } else {
                        self.stage = Stage::Tick;
                    }
                    return false;
                }
                Stage::Tick => {
                    draw_hp_bar(ui, self.at, BAR_TILES, self.pixels, false, self.kind);
                    self.stage = Stage::AfterTick;
                    self.wait = 2;
                    return false;
                }
                Stage::AfterTick => {
                    let delta = if self.target > self.old { 1 } else { u8::MAX };
                    self.pixels = self.pixels.wrapping_add(delta);
                    self.ticks -= 1;
                    // The bar stops early at 49 pixels, which on the way down is a wrap below 0.
                    if self.pixels > BAR_PIXELS || self.ticks == 0 {
                        self.old = self.next;
                        self.stage = Stage::Pass;
                    } else {
                        self.stage = Stage::Tick;
                    }
                }
                Stage::Last => {
                    draw_hp_bar(ui, self.at, BAR_TILES, self.pixels, false, self.kind);
                    self.stage = Stage::Done;
                    self.wait = 2;
                    return false;
                }
                Stage::Done => return true,
            }
        }
    }

    /// `UpdateHPBar_PrintHPNumber`: three tiles cleared and the HP it is leaving printed over them.
    fn print(&self, ui: &mut UiSurface) {
        let at = self.at + 9;
        ui.fill(at % SCREEN_TILES_X, at / SCREEN_TILES_X, 3, 1, UiSurface::BLANK);
        print_number(ui, at, self.old as u32, NumberFormat { digits: 3, leading_zeroes: false, left_align: false });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::cases;

    fn frames(mut animation: HpBarAnimation) -> (u32, UiSurface) {
        let mut ui = UiSurface::default();
        let mut frames = 1;
        while !animation.update(&mut ui) {
            frames += 1;
        }
        (frames, ui)
    }

    /// A point that moves no pixel costs its one printing frame; the end costs 1 + 2, and the
    /// routine returns in the frame after the last of them.
    #[test]
    fn a_point_is_a_frame_and_a_pixel_is_two_more() {
        let (flat, _) = frames(HpBarAnimation::new(0, 480, 100, 102, HpBarType::PartyMenu).unwrap());
        assert_eq!(flat, 2 + 3 + 1, "at 480 max HP two points move no pixel");
        let (steep, ui) = frames(HpBarAnimation::new(0, 48, 10, 12, HpBarType::PartyMenu).unwrap());
        assert_eq!(steep, 2 * 3 + 3 + 1, "at 48 max HP each point is a pixel");
        assert_eq!(ui.row(0)[11], 0xF6 + 2, "and the number ends on the new HP");
        assert_eq!(ui.row(0)[3], 0x63 + 4, "12 pixels: one full tile and four of the next");
    }

    #[test]
    fn every_harvested_case_of_hp_bar_length() {
        for (input, pixels, _) in cases::<HpBarInput, u8>(include_str!("../../fixtures/hp_bar/get_hp_bar_length.jsonl")) {
            assert_eq!(hp_bar_length(input.current, input.max), pixels, "{}/{}", input.current, input.max);
        }
    }

    #[test]
    fn a_full_bar_is_forty_eight_pixels_and_a_sliver_is_one() {
        assert_eq!(hp_bar_length(100, 100), 48);
        assert_eq!(hp_bar_length(1, 999), 1, "never rounds down to nothing");
        assert_eq!(hp_bar_length(50, 100), 24);
    }

    /// The colours the animation speeds are indexed by.
    #[test]
    fn the_bar_turns_yellow_below_27_pixels_and_red_below_10() {
        assert_eq!(health_bar_colour(27), HpBarColour::Green);
        assert_eq!(health_bar_colour(26), HpBarColour::Yellow);
        assert_eq!(health_bar_colour(10), HpBarColour::Yellow);
        assert_eq!(health_bar_colour(9), HpBarColour::Red);
        assert_eq!(HpBarColour::Red.animation_speed(), 32, "six times a green mon's");
    }

    #[test]
    fn the_party_menu_ends_its_bar_with_a_different_tile() {
        let mut ui = UiSurface::default();
        draw_hp_bar(&mut ui, 0, BAR_TILES, 48, true, HpBarType::PartyMenu);
        assert_eq!(ui.row(0)[..2], [0x71, 0x62]);
        assert_eq!(ui.row(0)[2..8], [0x6B; 6], "six full tiles");
        assert_eq!(ui.get(8, 0), 0x6C);
        let mut ui = UiSurface::default();
        draw_hp_bar(&mut ui, 0, BAR_TILES, 48, true, HpBarType::StatusScreenOrBattle);
        assert_eq!(ui.get(8, 0), 0x6D);
    }

    #[test]
    fn a_part_filled_tile_is_the_empty_tile_plus_its_pixels() {
        let mut ui = UiSurface::default();
        draw_hp_bar(&mut ui, 0, BAR_TILES, 11, false, HpBarType::PartyMenu);
        assert_eq!(ui.get(2, 0), 0x6B, "one full tile");
        assert_eq!(ui.get(3, 0), 0x63 + 3, "three pixels of the next");
        assert_eq!(ui.get(4, 0), 0x63, "and the rest empty");
    }

    #[test]
    fn a_fainted_mon_shows_no_sliver() {
        let mut ui = UiSurface::default();
        draw_hp(&mut ui, 0, 0, 100, true, HpBarType::PartyMenu);
        assert_eq!(ui.get(2, 0), 0x63, "empty, not a sliver");
    }
}
