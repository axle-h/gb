//! What the battle draws besides its texts: `DrawPlayerHUDAndHPBar`, `DrawEnemyHUDAndHPBar`, the
//! pictures, and `UpdateHPBar2` as the battle uses it.

use poke_core::rom_gfx::rom_slice;
use poke_core::species::PokemonSpecies;
use poke_core::symbols::pokered_symbols;
use serde::{Deserialize, Serialize};
use crate::gfx::tiles::V_CHARS2;
use crate::gfx::ui::{UiSurface, SCREEN_TILES_X};
use crate::systems::battle::{status, BattleMon};
use crate::systems::hp_bar::{draw_hp, draw_hp_bar, health_bar_colour, hp_bar_length, HpBarColour, HpBarType, BAR_TILES};
use crate::systems::math::{divide, multiply};
use crate::systems::pokedex::front_pic_tiles;
use crate::systems::print_num::{print_number, NumberFormat};
use crate::systems::status_screen::print_level;

pub const PIC_SIZE: usize = 7;
/// `hStartTileID` for the enemy's picture and `$31`, `vBackPic`, for the player's.
pub const FRONT_PIC_TILE: u8 = 0;
pub const BACK_PIC_TILE: u8 = 0x31;
const TERMINATOR: u8 = 0x50;

fn put(ui: &mut UiSurface, x: usize, y: usize, tile: u8) {
    ui.set(x, y, tile);
}

/// `ClearScreenArea`: `width` by `height` blanks from `(x, y)`.
pub fn clear_area(ui: &mut UiSurface, x: usize, y: usize, width: usize, height: usize) {
    ui.fill(x, y, width, height, UiSurface::BLANK);
}

/// `CopyUncompressedPicToHL`: seven columns of seven tile ids counting down each column from `first`.
pub fn place_pic(ui: &mut UiSurface, x: usize, y: usize, first: u8) {
    for column in 0..PIC_SIZE {
        for row in 0..PIC_SIZE {
            put(ui, x + column, y + row, first.wrapping_add((column * PIC_SIZE + row) as u8));
        }
    }
}

/// `LoadMonFrontSprite` into `vFrontPic`.
pub fn load_front_pic(tiles: &mut crate::gfx::tiles::TileData, species: PokemonSpecies) {
    tiles.load(V_CHARS2 + FRONT_PIC_TILE as usize, &front_pic_tiles(species, false).concat());
}

/// `ScaleSpriteByTwo` over a 4x4-tile back pic, then `InterlaceMergeSpriteBuffers` into
/// `vBackPic`: the top left 28 pixels square of the uncompressed pic, each pixel doubled, fills the
/// 7x7 tiles. `LoadMonBackPic` copies it to `vSprites` as well, which `LoadPlayerBackPic` does not.
fn load_scaled_back_pic(tiles: &mut crate::gfx::tiles::TileData, data: &'static [u8], to_sprites: bool) {
    use poke_core::mon_gfx::{pic_shades, PIC_PX};
    assert_eq!(data[0], 0x44, "a back pic is 4x4 tiles");
    // `pic_shades` centres the pic, two tiles in and three down.
    let centred = pic_shades(data);
    let mut scaled = [0u8; PIC_PX * PIC_PX];
    for y in 0..PIC_PX {
        for x in 0..PIC_PX {
            scaled[y * PIC_PX + x] = centred[(24 + y / 2) * PIC_PX + 16 + x / 2];
        }
    }
    let bytes = crate::systems::pokedex::pic_tiles(&scaled, false).concat();
    tiles.load(V_CHARS2 + BACK_PIC_TILE as usize, &bytes);
    if to_sprites {
        tiles.load(crate::gfx::tiles::V_CHARS0, &bytes);
    }
}

/// `LoadMonBackPic`'s picture.
pub fn load_back_pic(tiles: &mut crate::gfx::tiles::TileData, species: PokemonSpecies) {
    const BASE_BACKPIC: usize = 13;
    let entry = poke_core::mon_gfx::base_stats_entry(species);
    let address = u16::from_le_bytes([entry[BASE_BACKPIC], entry[BASE_BACKPIC + 1]]);
    let data = rom_slice(poke_core::symbols::DmgPointer { bank: pic_bank(species), address });
    load_scaled_back_pic(tiles, data, true);
}

/// `LoadPlayerBackPic`'s picture: the player's, or the old man's in his catching demo.
pub fn load_player_back_pic(tiles: &mut crate::gfx::tiles::TileData, old_man: bool) {
    let pic = if old_man { pokered_symbols::OldManPicBack } else { pokered_symbols::RedPicBack };
    load_scaled_back_pic(tiles, rom_slice(pic), false);
}

/// `UncompressMonSprite`'s bank ladder, on the internal index.
fn pic_bank(species: PokemonSpecies) -> poke_core::symbols::DmgBank {
    let bank = match species as u8 {
        _ if species == PokemonSpecies::Mew => 0x01,
        0x00..=0x1E => 0x09,
        0x1F..=0x49 => 0x0A,
        0x4A..=0x73 => 0x0B,
        0x74..=0x98 => 0x0C,
        _ => 0x0D,
    };
    poke_core::symbols::DmgBank::ROM { bank }
}

/// `SetupOwnPartyPokeballs` and `SetupEnemyPartyPokeballs`: the HUD's tiles, and a ball a mon
/// from OAM `first`, `step` pixels apart. `PickPokeball` gives `$31` for a mon that can fight, `$32`
/// for one with a status, `$33` for a fainted one, and `$34` for an empty slot.
pub fn place_pokeballs(sprites: &mut Vec<crate::gfx::layers::Object>, first: usize, (x, y): (u8, u8), step: i8,
                       party: &[(u16, u8)]) {
    if sprites.len() < 40 {
        sprites.resize(40, crate::gfx::layers::Object::default());
    }
    for slot in 0..6 {
        let tile = match party.get(slot) {
            None => 0x34,
            Some(&(0, _)) => 0x33,
            Some(&(_, 0)) => 0x31,
            Some(_) => 0x32,
        };
        let x = x.wrapping_add((step as u8).wrapping_mul(slot as u8));
        sprites[first + slot] = crate::gfx::layers::Object { y, x, tile, attributes: 0 };
    }
}

/// `LoadPartyPokeballGfx`, at `vSprites` tile `$31`.
pub fn load_pokeball_gfx(tiles: &mut crate::gfx::tiles::TileData) {
    let start = pokered_symbols::PokeballTileGraphics;
    let len = (pokered_symbols::PokeballTileGraphicsEnd.address - start.address) as usize;
    tiles.load(crate::gfx::tiles::V_CHARS0 + 0x31, &rom_slice(start)[..len]);
}

/// `_LoadTrainerPic` into `vFrontPic`.
pub fn load_trainer_pic(tiles: &mut crate::gfx::tiles::TileData, class: u8) {
    let (address, _) = poke_core::trainers::pic_and_money(class);
    let data = rom_slice(poke_core::symbols::DmgPointer { bank: pokered_symbols::YoungsterPic.bank, address });
    let shades = poke_core::mon_gfx::pic_shades(data);
    tiles.load(V_CHARS2 + FRONT_PIC_TILE as usize, &crate::systems::pokedex::pic_tiles(&shades, false).concat());
}

/// `LoadMonFrontSprite` for `MON_GHOST`, whose pic is `GhostPic`.
pub fn load_ghost_pic(tiles: &mut crate::gfx::tiles::TileData) {
    let shades = poke_core::mon_gfx::pic_shades(rom_slice(pokered_symbols::GhostPic));
    tiles.load(V_CHARS2 + FRONT_PIC_TILE as usize, &crate::systems::pokedex::pic_tiles(&shades, false).concat());
}

/// `PlaceEnemyHUDTiles`.
pub fn place_enemy_hud_tiles(ui: &mut UiSurface) {
    place_hud_tiles(ui, 1, 2, false, 0x74, 0x78);
}

/// `GetTrainerName` outside the rival's classes: the class's name from `TrainerNames`.
pub fn trainer_class_name(class: u8) -> Vec<u8> {
    rom_slice(pokered_symbols::TrainerNames).split(|&byte| byte == TERMINATOR)
        .nth(class as usize - 1).expect("every class has a name").to_vec()
}

/// `LoadHudTilePatterns`: the HUD's corners and lines, 1bpp, `BattleHudTiles1` at `$6D` and the
/// other two at `$73`.
pub fn load_hud_tiles(tiles: &mut crate::gfx::tiles::TileData) {
    let first = pokered_symbols::BattleHudTiles1;
    let len = (pokered_symbols::BattleHudTiles1End.address - first.address) as usize;
    tiles.load_1bpp(V_CHARS2 + 0x6D, &rom_slice(first)[..len]);
    let rest = pokered_symbols::BattleHudTiles2;
    let len = (pokered_symbols::BattleHudTiles3End.address - rest.address) as usize;
    tiles.load_1bpp(V_CHARS2 + 0x73, &rom_slice(rest)[..len]);
}

/// `CenterMonName`: a name of one or two letters two tiles right, three or four one tile right.
fn centred(x: usize, name: &[u8]) -> usize {
    let len = name.iter().take_while(|&&byte| byte != TERMINATOR).count();
    match len {
        0..=2 => x + 2,
        3 | 4 => x + 1,
        _ => x,
    }
}

/// `PrintStatusConditionNotFainted`: false when there is nothing to print, the cartridge's zero flag.
fn print_status_ailment(ui: &mut UiSurface, x: usize, y: usize, status_byte: u8) -> bool {
    let text: &[u8] = match status_byte {
        s if s & status::PSN != 0 => &[0x8F, 0x92, 0x8D],
        s if s & status::BRN != 0 => &[0x81, 0x91, 0x8D],
        s if s & status::FRZ != 0 => &[0x85, 0x91, 0x99],
        s if s & status::PAR != 0 => &[0x8F, 0x80, 0x91],
        s if s & status::SLP_MASK != 0 => &[0x92, 0x8B, 0x8F],
        _ => return false,
    };
    ui.place(x, y, text);
    true
}

/// `PlaceHUDTiles`: the end tile, the corner under it, eight lines and the triangle, running left for
/// the player and right for the enemy.
fn place_hud_tiles(ui: &mut UiSurface, x: usize, y: usize, leftward: bool, corner: u8, triangle: u8) {
    put(ui, x, y, 0x73);
    put(ui, x, y + 1, corner);
    let step = |i: usize| if leftward { x - i } else { x + i };
    for i in 1..=8 {
        put(ui, step(i), y + 1, 0x76);
    }
    put(ui, step(9), y + 1, triangle);
}

/// `PlacePlayerHUDTiles`.
pub fn place_player_hud_tiles(ui: &mut UiSurface) {
    place_hud_tiles(ui, 18, 10, true, 0x77, 0x6F);
}

/// `DrawPlayerHUDAndHPBar`, returning the bar's colour for `wPlayerHPBarColor`.
pub fn draw_player_hud(ui: &mut UiSurface, mon: &BattleMon, nick: &[u8]) -> HpBarColour {
    clear_area(ui, 9, 7, 11, 5);
    place_hud_tiles(ui, 18, 10, true, 0x77, 0x6F);
    put(ui, 18, 9, 0x73);
    ui.place(centred(10, nick), 7, nick);
    if !print_status_ailment(ui, 15, 8, mon.status) {
        print_level(ui, 14 + 8 * SCREEN_TILES_X, mon.level);
    }
    draw_hp(ui, 10 + 9 * SCREEN_TILES_X, mon.hp, mon.stats[0], false, HpBarType::StatusScreenOrBattle)
}

/// `DrawEnemyHUDAndHPBar`, returning the bar's colour for `wEnemyHPBarColor`. The bar is worked out
/// here rather than by `GetHPBarLength`, so a mon with HP can come out with no pixels, which the bar
/// then draws as a sliver.
pub fn draw_enemy_hud(ui: &mut UiSurface, mon: &BattleMon, nick: &[u8]) -> HpBarColour {
    clear_area(ui, 0, 0, 12, 4);
    place_hud_tiles(ui, 1, 2, false, 0x74, 0x78);
    ui.place(centred(1, nick), 0, nick);
    if !print_status_ailment(ui, 5, 1, mon.status) {
        print_level(ui, 4 + SCREEN_TILES_X, mon.level);
    }
    let pixels = enemy_hp_pixels(mon.hp, mon.stats[0]);
    draw_hp_bar(ui, 2 + 2 * SCREEN_TILES_X, BAR_TILES, pixels, mon.hp != 0, HpBarType::PartyMenu);
    health_bar_colour(pixels)
}

/// `DrawEnemyHUDAndHPBar`'s `current * 48 / max`, with no floor of one pixel.
fn enemy_hp_pixels(hp: u16, max: u16) -> u8 {
    if hp == 0 {
        return 0;
    }
    let product = multiply(hp as u32, 48).to_be_bytes();
    let (dividend, divisor) = if max > 0xFF {
        let low = u16::from_be_bytes([product[2], product[3]]) >> 2;
        (low.to_be_bytes(), (max >> 2) as u8)
    } else {
        ([product[2], product[3]], max as u8)
    };
    divide([dividend[0], dividend[1], 0, 0], divisor, 2).0[3]
}

/// Which bar `UpdateHPBar2` moves: `wHPBarType` 0 for the enemy, with no number, and 1 for the
/// player, whose number is under the bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Bar {
    Enemy,
    Player,
}

impl Bar {
    fn at(self) -> usize {
        match self {
            Bar::Enemy => 2 + 2 * SCREEN_TILES_X,
            Bar::Player => 10 + 9 * SCREEN_TILES_X,
        }
    }

    fn kind(self) -> HpBarType {
        match self {
            Bar::Enemy => HpBarType::PartyMenu,
            Bar::Player => HpBarType::StatusScreenOrBattle,
        }
    }
}

/// `UpdateHPBar2` in battle: a pass a point, printing the HP it leaves (the player's only, and only
/// the player's waits a frame for it) and two frames a pixel moved; then the new HP printed and the
/// bar drawn once more with two frames after.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HpBar {
    bar: Bar,
    max: u16,
    old: u16,
    target: u16,
    /// Pixels drawn, and the ticks left of the pass under way.
    pixels: u8,
    ticks: u8,
    wait: u8,
    stage: Stage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Pass,
    Tick { next: u16 },
    Last,
    Done,
}

impl HpBar {
    /// `None` where the HP does not change, which returns at once.
    pub fn new(bar: Bar, max: u16, old: u16, new: u16) -> Option<Self> {
        (old != new).then_some(Self { bar, max, old, target: new, pixels: 0, ticks: 0, wait: 0, stage: Stage::Pass })
    }

    fn print(&self, ui: &mut UiSurface) -> u8 {
        if self.bar == Bar::Enemy {
            return 0;
        }
        let at = self.bar.at() + 0x15;
        ui.fill(at % SCREEN_TILES_X, at / SCREEN_TILES_X, 3, 1, UiSurface::BLANK);
        print_number(ui, at, self.old as u32, NumberFormat { digits: 3, leading_zeroes: false, left_align: false });
        1
    }

    /// One frame; true in the frame the routine returns.
    pub fn update(&mut self, ui: &mut UiSurface) -> bool {
        loop {
            if self.wait > 0 {
                self.wait -= 1;
                if self.wait > 0 {
                    return false;
                }
            }
            match self.stage {
                Stage::Pass if self.old == self.target => {
                    // `GetHPBarLength` floors at one pixel, but `.monFainted` skips it and draws none,
                    // except where the number was printed: `PrintNumber` leaves 3 in `c`, which
                    // `DrawHPBar` takes as asking for a sliver.
                    self.pixels = match (self.target, self.bar) {
                        (0, Bar::Enemy) => 0,
                        (0, Bar::Player) => 1,
                        (target, _) => hp_bar_length(target, self.max),
                    };
                    self.stage = Stage::Last;
                    self.wait = self.print(ui);
                    if self.wait > 0 {
                        return false;
                    }
                }
                Stage::Pass => {
                    let next = if self.target > self.old { self.old + 1 } else { self.old - 1 };
                    let (from, to) = (hp_bar_length(self.old, self.max), hp_bar_length(next, self.max));
                    self.pixels = from;
                    self.ticks = from.abs_diff(to);
                    self.stage = if self.ticks == 0 { self.old = next; Stage::Pass } else { Stage::Tick { next } };
                    self.wait = self.print(ui);
                    if self.wait > 0 {
                        return false;
                    }
                }
                Stage::Tick { next } => {
                    draw_hp_bar(ui, self.bar.at(), BAR_TILES, self.pixels, false, self.bar.kind());
                    let delta = if self.target > self.old { 1 } else { u8::MAX };
                    self.pixels = self.pixels.wrapping_add(delta);
                    self.ticks -= 1;
                    if self.pixels > 48 || self.ticks == 0 {
                        self.old = next;
                        self.stage = Stage::Pass;
                    }
                    self.wait = 2;
                    return false;
                }
                Stage::Last => {
                    draw_hp_bar(ui, self.bar.at(), BAR_TILES, self.pixels, false, self.bar.kind());
                    self.stage = Stage::Done;
                    self.wait = 2;
                    return false;
                }
                Stage::Done => return true,
            }
        }
    }
}
