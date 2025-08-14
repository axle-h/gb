use std::collections::BTreeMap;
use crate::cycles::MachineCycles;
use crate::geometry::Point8;
use crate::interrupt::InterruptSource;
use crate::lcd_control::{LcdControl, TileDataMode};
use crate::lcd_dma::LcdDma;
use crate::lcd_palette::{DMGColor, LcdPalette};
use crate::lcd_status::{LcdMode, LcdStatus};

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
    current_x: usize,
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
            current_x: 0
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
                }
                // TODO get current scanline sprite data here
            }
            LcdMode::Drawing => {
                // TODO properly emulate FIFO
                let drawing_ticks = INITIAL_FIFO_LOAD_TICKS + LCD_WIDTH;

                if self.current_ticks >= drawing_ticks {
                    self.lcd_status.set_mode(LcdMode::HBlank); // drawing done
                    self.current_ticks -= drawing_ticks;
                } else if self.current_ticks >= INITIAL_FIFO_LOAD_TICKS {
                    // shifting pixels
                    let start_x = self.current_x;
                    let end_x = start_x + self.current_ticks - INITIAL_FIFO_LOAD_TICKS + 1;
                    let y = self.lcd_status.ly() as usize;
                    for x in start_x..end_x {
                        if x < LCD_WIDTH {
                            self.draw_pixel(x, y);
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
                    }
                }
            }
        }
    }

    fn tile(&self, index: u8) -> Tile {
        let address = self.lcd_control.tile_data_mode().tile_address(index) as usize - VRAM_BASE_ADDRESS;
        Tile::new(&self.vram[address..address + TILE_BYTES])
    }

    fn object_tile(&self, index: u8) -> Tile {
        let address = TileDataMode::Lower.tile_address(index) as usize - VRAM_BASE_ADDRESS;
        Tile::new(&self.vram[address..address + TILE_BYTES])
    }

    fn background_tile_map(&self) -> &[u8] {
        let address = self.lcd_control.background_tile_map().base_address() as usize - VRAM_BASE_ADDRESS;
        &self.vram[address..address + TILE_MAP_BYTES]
    }

    fn window_tile_map(&self) -> &[u8] {
        let address = self.lcd_control.window_tile_map().base_address() as usize - VRAM_BASE_ADDRESS;
        &self.vram[address..address + TILE_MAP_BYTES]
    }

    fn draw_pixel(&mut self, x: usize, y: usize) {
        let tilemap_point = (Point8 { x: x as u8, y: y as u8 } + self.scroll) / TILE_PIXELS as u8;
        let bg_tile_map_address = self.lcd_control.background_tile_map().base_address() as usize - VRAM_BASE_ADDRESS;
        let bg_tile_index = self.vram[bg_tile_map_address + tilemap_point.y as usize * TILE_MAP_SIZE + tilemap_point.x as usize];
        let tile = self.tile(bg_tile_index);
        self.lcd[y * LCD_WIDTH + x] = tile.pixel(x % TILE_PIXELS, y % TILE_PIXELS);
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
struct Tile<'a>(&'a [u8]);

impl<'a> Tile<'a> {
    fn new(data: &'a [u8]) -> Self {
        debug_assert!(data.len() == TILE_BYTES, "Tile data must be exactly 16 bytes");
        Self(data)
    }

    pub fn pixel(&self, x: usize, y: usize) -> DMGColor {
        debug_assert!(x < TILE_PIXELS && y < TILE_PIXELS, "Coordinates out of bounds for tile");
        let byte1 = self.0[y * 2];
        let byte2 = self.0[y * 2 + 1];
        let color_index = ((byte1 >> (7 - x)) & 1) | (((byte2 >> (7 - x)) & 1) << 1);
        DMGColor::from_repr(color_index).unwrap_or(DMGColor::White)
    }

    pub fn line(&self, y: usize) -> [DMGColor; TILE_PIXELS] {
        debug_assert!(y < TILE_PIXELS, "Line index out of bounds for tile");
        let mut line = [DMGColor::White; TILE_PIXELS];
        for x in 0..TILE_PIXELS {
            line[x] = self.pixel(x, y);
        }
        line
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