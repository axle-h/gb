use std::collections::BTreeMap;
use crate::cycles::MachineCycles;
use crate::geometry::Point8;
use crate::interrupt::InterruptSource;
use crate::lcd_control::{LcdControl, ObjectSizeMode, TileDataMode, TileMapMode};
use crate::lcd_dma::LcdDma;
use crate::lcd_palette::{DMGColor, DMGPaletteRegister, LcdPalette};
use crate::lcd_status::{LcdMode, LcdStatus};
use image::{ImageBuffer, Rgb, RgbImage};
use itertools::Itertools;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PPU {
    vram: [u8; 0x2000], // 8KB VRAM
    oam: [u8; 0xA0], // 160 bytes OAM (Object Attribute Memory)
    lcd_control: LcdControl,
    lcd_status: LcdStatus,
    vblank_interrupt_pending: bool,
    scroll: Point8,
    window_position: Point8,
    palette: LcdPalette,
    dma: LcdDma,
    lcd: [DMGColor; LCD_WIDTH * LCD_HEIGHT],
    current_ticks: usize, // Current machine cycles

    // TODO move all these into a separate struct for the current frame state
    current_x: usize,
    window_state: WindowRenderState,
    scanline_sprites: Vec<Sprite>
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WindowRenderState {
    is_active: bool,
    max_y: usize,
    window_y: usize,
}

impl WindowRenderState {
    pub fn update_if_active(&mut self, y: usize) {
        if self.is_active {
            if y > self.max_y {
                self.window_y += 1;
                self.max_y = y;
            }
        }
    }

    pub fn activate(&mut self, y: usize, window_position: Point8) {
        self.is_active = true;
        self.max_y = y;
        self.window_y = y - window_position.y as usize;
    }

    pub fn deactivate(&mut self) {
        self.is_active = false;
        self.max_y = 0;
        self.window_y = 0;
    }
}

impl Default for PPU {
    fn default() -> Self {
        Self {
            vram: [0; 0x2000],
            oam: [0; 0xA0],
            lcd_control: LcdControl::default(),
            lcd_status: LcdStatus::default(),
            vblank_interrupt_pending: false,
            scroll: Point8::default(),
            window_position: Point8::default(),
            palette: LcdPalette::default(),
            dma: LcdDma::default(),
            lcd: [DMGColor::White; LCD_WIDTH * LCD_HEIGHT],
            current_ticks: 0,
            current_x: 0,
            window_state: WindowRenderState::default(),
            scanline_sprites: vec![],
        }
    }
}

impl PPU {
    pub fn lcd(&self) -> &[DMGColor; LCD_WIDTH * LCD_HEIGHT] {
        &self.lcd
    }

    pub fn read_vram(&self, address: u16) -> u8 {
        if self.lcd_status.mode().vram_accessible() || self.dma.is_active() {
            self.vram[address as usize]
        } else {
            // garbage data https://gbdev.io/pandocs/Rendering.html
            0xff
        }
    }

    pub fn write_vram(&mut self, address: u16, value: u8) {
        if self.lcd_status.mode().vram_accessible() || self.dma.is_active() {
            self.vram[address as usize] = value;
        }
    }

    pub fn read_oam(&self, address: u16) -> u8 {
        if self.lcd_status.mode().oam_accessible() || self.dma.is_active() {
            self.oam[address as usize]
        } else {
            // garbage data https://gbdev.io/pandocs/Rendering.html
            0xff
        }
    }

    pub fn write_oam(&mut self, address: u16, value: u8) {
        if self.lcd_status.mode().oam_accessible() || self.dma.is_active() {
            self.oam[address as usize] = value;
        }
    }

    pub fn lcd_control(&self) -> &LcdControl {
        &self.lcd_control
    }

    pub fn lcd_control_mut(&mut self) -> &mut LcdControl {
        &mut self.lcd_control
    }

    pub fn lcd_status(&self) -> &LcdStatus {
        &self.lcd_status
    }

    pub fn lcd_status_mut(&mut self) -> &mut LcdStatus {
        &mut self.lcd_status
    }

    pub fn scroll(&self) -> &Point8 {
        &self.scroll
    }

    pub fn scroll_mut(&mut self) -> &mut Point8 {
        &mut self.scroll
    }

    pub fn window_position(&self) -> &Point8 {
        &self.window_position
    }

    pub fn window_position_mut(&mut self) -> &mut Point8 {
        &mut self.window_position
    }

    pub fn palette(&self) -> &LcdPalette {
        &self.palette
    }

    pub fn palette_mut(&mut self) -> &mut LcdPalette {
        &mut self.palette
    }

    pub fn dma(&self) -> &LcdDma {
        &self.dma
    }

    pub fn dma_mut(&mut self) -> &mut LcdDma {
        &mut self.dma
    }

    /// Generate a screenshot of the current PPU state as an in-memory RGB image
    pub fn screenshot(&self) -> RgbImage {
        let mut img = ImageBuffer::new(LCD_WIDTH as u32, LCD_HEIGHT as u32);
        for y in 0..LCD_HEIGHT {
            for x in 0..LCD_WIDTH {
                let rgb_color = self.lcd[y * LCD_WIDTH + x].to_rgb();
                img.put_pixel(x as u32, y as u32, rgb_color);
            }
        }
        img
    }

    pub fn dump_tilemap(&self, tile_map_mode: TileMapMode, data_mode: TileDataMode) -> RgbImage {
        let tile_map = self.tile_map(tile_map_mode);
        let mut img = ImageBuffer::new(TILE_MAP_PIXELS as u32, TILE_MAP_PIXELS as u32);
        for y in 0..TILE_MAP_PIXELS {
            for x in 0..TILE_MAP_PIXELS {
                let color_index = self.pixel(&tile_map, data_mode, x, y);
                let pixel_color = DMGColor::from_repr(color_index).unwrap_or(DMGColor::White).to_rgb();
                img.put_pixel(x as u32, y as u32, pixel_color);
            }
        }
        img
    }

    pub fn update(&mut self, delta_machine_cycles: MachineCycles) {
        if !self.lcd_control.is_enabled() {
            // TODO should the screen be blanked?
            return
        }

        self.current_ticks += delta_machine_cycles.to_ticks(); // TODO the PPU is twice as slow in CGB double speed mode

        match self.lcd_status.mode() {
            LcdMode::OAM => {
                if self.current_ticks >= OAM_TICKS {
                    self.lcd_status.set_mode(LcdMode::Drawing);
                    self.current_ticks -= OAM_TICKS;

                    let y = self.lcd_status.ly() as isize;
                    let sprite_height = self.lcd_control.object_size().height() as isize;
                    self.scanline_sprites = if self.lcd_control.objects_enabled() {
                        self.sprites().into_iter()
                            .filter(|sprite| y >= sprite.y && y < sprite.y + sprite_height)
                            .take(MAX_SPRITES_PER_SCANLINE)
                            .collect()
                    } else {
                        vec![]
                    }
                }
            }
            LcdMode::Drawing => {
                let drawing_ticks = INITIAL_FIFO_LOAD_TICKS + LCD_WIDTH;

                if self.current_ticks >= drawing_ticks {
                    self.lcd_status.set_mode(LcdMode::HBlank); // drawing done
                    self.current_ticks -= drawing_ticks;
                } else if self.current_ticks >= INITIAL_FIFO_LOAD_TICKS {
                    let start_x = self.current_x;
                    let end_x = start_x + self.current_ticks - INITIAL_FIFO_LOAD_TICKS + 1;
                    let y = self.lcd_status.ly() as usize;

                    if self.lcd_status.ly() == self.window_position.y && !self.window_state.is_active {
                        self.window_state.activate(y, self.window_position);
                    }

                    let mut row_in_window = false;
                    for x in start_x..end_x {
                        if x < LCD_WIDTH {
                            let pixel_in_window = self.in_window(x, y);
                            if pixel_in_window && !row_in_window {
                                row_in_window = true;
                                self.window_state.update_if_active(y);
                            }

                            let bg_color_index = if pixel_in_window {
                                self.window_pixel(x)
                            } else if self.lcd_control.background_enabled() {
                                self.bg_pixel(x, y)
                            } else {
                                0
                            } as usize;
                            let bg_color = self.palette.background()[bg_color_index];

                            let color = self.scanline_sprites.iter()
                                .filter(|sprite| sprite.x <= x as isize && sprite.x + TILE_PIXELS as isize > x as isize)
                                .map(|sprite| (sprite, self.sprite_pixel(sprite, x, y)))
                                .filter(|&(_, sprite_color)| sprite_color != 0) // filter out transparent pixels
                                .sorted_by_key(|&(sprite, _)| sprite.x) // overlapping sprites are sorted by x position
                                .next()
                                .map_or(bg_color, |(sprite, sprite_color)| {
                                    if sprite_color == 0 || sprite.bg_priority && bg_color_index != 0 {
                                        bg_color
                                    } else {
                                        sprite.palette(&self.palette)[sprite_color as usize]
                                    }
                                });

                            self.lcd[y * LCD_WIDTH + x] = color;
                        }
                    }
                    self.current_x = end_x;
                }
            }
            LcdMode::HBlank => {
                // TODO vary the length of the HBlank period based on the length of the Drawing phase
                let hblank_ticks = SCANLINE_TICKS - OAM_TICKS - INITIAL_FIFO_LOAD_TICKS - LCD_WIDTH;
                if self.current_ticks >= hblank_ticks {
                    // hblank finished, go to next scanline
                    self.current_ticks -= hblank_ticks;
                    self.current_x = 0; // reset X for the next scanline
                    let next_ly = self.lcd_status.increment_ly();

                    if next_ly >= LCD_HEIGHT as u8 {
                        // Enter VBlank mode
                        self.vblank_interrupt_pending = true;
                        self.lcd_status.set_mode(LcdMode::VBlank);
                    } else {
                        // Continue to OAM mode for the next scanline
                        self.lcd_status.set_mode(LcdMode::OAM);
                    }
                }
            }
            LcdMode::VBlank => {
                if self.current_ticks >= SCANLINE_TICKS {
                    self.current_ticks -= SCANLINE_TICKS;
                    let next_ly = self.lcd_status.increment_ly();
                    if next_ly == 0 {
                        // VBlank finished, reset to OAM mode
                        self.lcd_status.set_mode(LcdMode::OAM);
                        self.window_state.deactivate();
                    }
                }
            }
        }
    }

    fn tile(&self, mode: TileDataMode, index: u8) -> Tile {
        let address = mode.tile_address(index) as usize - VRAM_BASE_ADDRESS;
        Tile::new(&self.vram[address..address + TILE_BYTES])
    }

    fn tile_map(&self, tilemap_mode: TileMapMode) -> TileMap {
        let address = tilemap_mode.base_address() as usize - VRAM_BASE_ADDRESS;
        TileMap(&self.vram[address..address + TILE_MAP_BYTES])
    }

    /// After each pixel shifted out, the PPU checks if it has reached the window. It does this by checking the following conditions:
    ///     Bit 5 of the LCDC register is set to 1
    ///     The condition WY = LY has been true at any point in the currently rendered frame.
    ///     The current X-position of the shifter is greater than or equal to WX - 7
    fn in_window(&self, x: usize, y: usize) -> bool {
        self.lcd_control.window_enabled() &&
            self.window_state.is_active &&
            x >= self.window_position.x.saturating_sub(7) as usize
    }

    fn window_pixel(&self, x: usize) -> u8 {
        let tile_map = self.tile_map(self.lcd_control.window_tile_map());
        self.pixel(
            &tile_map,
            self.lcd_control.tile_data_mode(),
            // x+7 because window starts at x position - 7
            x + 7 - self.window_position.x as usize,
            // the y coordinate is derived from the total number of window lines rendered
            self.window_state.window_y
        )
    }

    fn bg_pixel(&self, x: usize, y: usize) -> u8 {
        let tile_map = self.tile_map(self.lcd_control.background_tile_map());
        self.pixel(
            &tile_map,
            self.lcd_control.tile_data_mode(),
            (x as u8).wrapping_add(self.scroll.x) as usize,
            (y as u8).wrapping_add(self.scroll.y) as usize
        )
    }

    fn sprite_pixel(&self, sprite: &Sprite, x: usize, y: usize) -> u8 {
        let object_size = self.lcd_control.object_size();
        let sprite_x = (x as isize - sprite.x) as usize;
        let pixel_x = if sprite.flip_x { TILE_PIXELS - 1 - sprite_x } else { sprite_x };
        let sprite_y = (y as isize - sprite.y) as usize;
        let pixel_y = if sprite.flip_y { object_size.height() - 1 - sprite_y } else { sprite_y };

        match object_size {
            ObjectSizeMode::Single => self.tile(TileDataMode::Lower, sprite.tile_index).pixel(pixel_x, pixel_y),
            ObjectSizeMode::Double => {
                if pixel_y < TILE_PIXELS {
                    self.tile(TileDataMode::Lower, sprite.tile_index & 0xFE)
                        .pixel(pixel_x, pixel_y)
                } else {
                    self.tile(TileDataMode::Lower, sprite.tile_index | 0x01)
                        .pixel(pixel_x, pixel_y - TILE_PIXELS)
                }
            }
        }
    }

    fn pixel(&self, tile_map: &TileMap, data_mode: TileDataMode, x: usize, y: usize) -> u8 {
        let tile_index = tile_map.tile_index(x / TILE_PIXELS, y / TILE_PIXELS);
        let tile = self.tile(data_mode, tile_index);
        tile.pixel(x % TILE_PIXELS, y % TILE_PIXELS)
    }



    fn sprites(&self) -> Vec<Sprite> {
        let mut sprites = Vec::with_capacity(SPRITE_COUNT);
        for i in 0..SPRITE_COUNT {
            let start = i * SPRITE_BYTES;
            sprites.push(Sprite::new(&self.oam[start..start + SPRITE_BYTES]));
        }
        sprites
    }
}


const VRAM_BASE_ADDRESS: usize = 0x8000;
pub const LCD_WIDTH: usize = 160;
pub const LCD_HEIGHT: usize = 144;
pub const TILE_BYTES: usize = 16;
const TILE_PIXELS: usize = 8;
const TILE_MAP_SIZE: usize = 32;
const TILE_MAP_BYTES: usize = TILE_MAP_SIZE * TILE_MAP_SIZE;
const TILE_MAP_PIXELS: usize = TILE_MAP_SIZE * TILE_PIXELS; // 256 pixels
const SPRITE_BYTES: usize = 4;
const SPRITE_COUNT: usize = 40;
const MAX_SPRITES_PER_SCANLINE: usize = 10;

const OAM_TICKS: usize = 80;
const INITIAL_FIFO_LOAD_TICKS: usize = 12;
const SCANLINE_TICKS: usize = 456;

impl InterruptSource for PPU {
    fn is_interrupt_pending(&self) -> bool {
        self.vblank_interrupt_pending
    }

    fn clear_interrupt(&mut self) {
        self.vblank_interrupt_pending = false;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TileMap<'a>(&'a [u8]);

impl<'a> TileMap<'a> {
    fn new(data: &'a [u8]) -> Self {
        debug_assert!(data.len() == TILE_MAP_BYTES, "Tile map data must be exactly 1024 bytes");
        Self(data)
    }

    pub fn tile_index(&self, x: usize, y: usize) -> u8 {
        debug_assert!(x < TILE_MAP_SIZE && y < TILE_MAP_SIZE, "Coordinates out of bounds for tile map");
        self.0[y * TILE_MAP_SIZE + x]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Tile<'a>(&'a [u8]);

impl<'a> Tile<'a> {
    fn new(data: &'a [u8]) -> Self {
        debug_assert!(data.len() == TILE_BYTES, "Tile data must be exactly 16 bytes");
        Self(data)
    }

    pub fn pixel(&self, x: usize, y: usize) -> u8 {
        debug_assert!(x < TILE_PIXELS && y < TILE_PIXELS, "Coordinates out of bounds for tile");
        let byte1 = self.0[y * 2];
        let byte2 = self.0[y * 2 + 1];
        ((byte1 >> (7 - x)) & 1) | (((byte2 >> (7 - x)) & 1) << 1)
    }

    pub fn line(&self, y: usize) -> [DMGColor; TILE_PIXELS] {
        debug_assert!(y < TILE_PIXELS, "Line index out of bounds for tile");
        let mut line = [DMGColor::White; TILE_PIXELS];
        for x in 0..TILE_PIXELS {
            line[x] = DMGColor::from_repr(self.pixel(x, y)).unwrap_or(DMGColor::White);
        }
        line
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Sprite {
    y: isize,
    x: isize,
    tile_index: u8,
    bg_priority: bool, // bit 7 - 0 = No, 1 = BG and Window color indices 1–3 are drawn over this OBJ
    flip_y: bool, // bit 6 - 0 = Normal, 1 = Entire OBJ is vertically mirrored
    flip_x: bool, // bit 5 - 0 = Normal, 1 = Entire OBJ is horizontally mirrored
    alt_palette: bool, // bit 4 (DMG only) - 0 = Use OBJ palette 0, 1 = Use OBJ palette 1
}

impl Sprite {
    pub fn new(data: &[u8]) -> Self {
        debug_assert!(data.len() == SPRITE_BYTES, "Sprite data must be exactly 4 bytes");
        Self {
            y: data[0] as isize - 16, // Y coordinate is offset by 16 pixels
            x: data[1] as isize - 8, // X coordinate is offset by 8 pixels
            tile_index: data[2],
            bg_priority: (data[3] & 0x80) != 0,
            flip_y: (data[3] & 0x40) != 0,
            flip_x: (data[3] & 0x20) != 0,
            alt_palette: (data[3] & 0x10) != 0,
        }
    }

    pub fn palette<'a>(&self, register: &'a LcdPalette) -> &'a DMGPaletteRegister {
        if self.alt_palette {
            register.object1()
        } else {
            register.object0()
        }
    }
}


#[cfg(test)]
mod tests {
    use DMGColor::*;
    use super::*;

    #[test]
    fn parse_tile() {
        let tile = Tile::new(&[
            0x3C, 0x7E, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x7E, 0x5E, 0x7E, 0x0A, 0x7C, 0x56, 0x38, 0x7C
        ]);
        assert_eq!(
            tile.line(0),
            [White, DarkGray, Black, Black, Black, Black, DarkGray, White]
        );
        assert_eq!(
            tile.line(1),
            [White, Black, White, White, White, White, Black, White]
        );
        assert_eq!(
            tile.line(2),
            [White, Black, White, White, White, White, Black, White]
        );
        assert_eq!(
            tile.line(3),
            [White, Black, White, White, White, White, Black, White]
        );
        assert_eq!(
            tile.line(4),
            [White, Black, LightGray, Black, Black, Black, Black, White]
        );
        assert_eq!(
            tile.line(5),
            [White, LightGray, LightGray, LightGray, Black, LightGray, Black, White]
        );
        assert_eq!(
            tile.line(6),
            [White, Black, LightGray, Black, LightGray, Black, DarkGray, White]
        );
        assert_eq!(
            tile.line(7),
            [White, DarkGray, Black, Black, Black, DarkGray, White, White]
        );
    }
}