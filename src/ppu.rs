use std::collections::{BTreeMap, HashMap, HashSet};
use bincode::{Decode, Encode};
use crate::cgb_palette::{PaletteBank, PaletteBankState};
use crate::cycles::MachineCycles;
use crate::geometry::Point8;
use crate::activation::Activation;
use crate::lcd_control::{LcdControl, ObjectSizeMode, TileDataMode, TileMapMode};
use crate::lcd_dma::LcdDma;
use crate::lcd_palette::{DMGColor, DMGPaletteRegister, LcdColor, LcdPalette};
use crate::lcd_status::{LcdMode, LcdStatus};
use crate::model::ColorMode;
use crate::savestate::{labels, SectionReader, SectionWriter};
use image::{ImageBuffer, Rgb, RgbImage};
use itertools::Itertools;

#[derive(Debug, Clone)]
pub struct PPU {
    /// Two 8 KB banks. A DMG only ever uses bank 0, and both [`PPU::vram`] and the CPU-facing
    /// accessors keep that indistinguishable from the old single-bank array.
    vram: [u8; VRAM_BANK_SIZE * VRAM_BANKS],
    /// `VBK` (`FF4F`), 0 or 1. Always 0 on DMG and in CGB compatibility mode.
    vram_bank: usize,
    oam: [u8; 0xA0], // 160 bytes OAM (Object Attribute Memory)
    lcd_control: LcdControl,
    lcd_status: LcdStatus,
    vblank_interrupt_pending: bool,
    scroll: Point8,
    window_position: Point8,
    palette: LcdPalette,
    /// CGB background palette RAM (`BCPS`/`BCPD`). Unused on DMG; in compatibility mode it is
    /// pre-loaded by the boot ROM and read-only to the cartridge.
    cgb_background: PaletteBank,
    /// CGB object palette RAM (`OCPS`/`OCPD`).
    cgb_object: PaletteBank,
    /// `OPRI` (`FF6C`): bit 0 set selects DMG sprite priority (lowest X wins) instead of CGB's
    /// (lowest OAM index wins). The boot ROM sets it in compatibility mode (`EmulateDMG`).
    object_priority: u8,
    dma: LcdDma,
    lcd: [LcdColor; LCD_WIDTH * LCD_HEIGHT],
    current_ticks: usize, // Current machine cycles
    /// Set on each mode 3 -> mode 0 transition and consumed by the MMU in the same `update` call,
    /// which is what paces HDMA. Never observable at rest, so it is neither serialised nor part
    /// of equality.
    hblank_started: bool,
    /// Which palette hardware drives the screen. A construction-time property, not guest state.
    color_mode: ColorMode,

    // TODO move all these into a separate struct for the current frame state
    current_x: usize,
    window_state: WindowRenderState,
    /// The (at most ten) sprites the OAM scan selected for the current scanline, **in OAM order**
    /// — which is the CGB priority order. A fixed array rather than a `Vec`: the limit is a hard
    /// ten, and the allocation was ~8640 of them a second (C5).
    scanline_sprites: [Sprite; MAX_SPRITES_PER_SCANLINE],
    scanline_sprite_count: usize,
    /// The same sprites indexed in **DMG priority order**: by X, ties by OAM index. Computed once
    /// per scanline, because deriving it per pixel is what `sorted_by_key` used to do — inside the
    /// innermost pixel loop, allocating a `Vec` each time (C5).
    scanline_sprite_order: [u8; MAX_SPRITES_PER_SCANLINE],
    /// One bit per screen column: is any selected sprite over it at all?
    ///
    /// `top_sprite` is a scan, and without this it runs for every pixel of every scanline — including
    /// the great majority that no sprite is anywhere near. Ten sprites can cover at most 80 of the
    /// 160 columns, so this skips the scan outright more often than not.
    scanline_sprite_columns: [u64; LCD_WIDTH.div_ceil(64)],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Decode, Encode)]
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

/// `lcd` is derived output and `scanline_sprites` is rebuilt from OAM on every scanline. Neither
/// is machine state, so neither is serialised — which means neither may take part in equality
/// either, or the save/load round-trip assertion in `game_boy::tests::save_and_load_state` would
/// compare a restored frame buffer against one that was never saved. `Audio` excludes its
/// resampler output for exactly the same reason.
impl PartialEq for PPU {
    fn eq(&self, other: &Self) -> bool {
        self.vram == other.vram
            && self.vram_bank == other.vram_bank
            && self.oam == other.oam
            && self.lcd_control == other.lcd_control
            && self.lcd_status == other.lcd_status
            && self.vblank_interrupt_pending == other.vblank_interrupt_pending
            && self.scroll == other.scroll
            && self.window_position == other.window_position
            && self.palette == other.palette
            && self.cgb_background == other.cgb_background
            && self.cgb_object == other.cgb_object
            && self.object_priority == other.object_priority
            && self.dma == other.dma
            && self.current_ticks == other.current_ticks
            && self.color_mode == other.color_mode
            && self.current_x == other.current_x
            && self.window_state == other.window_state
    }
}

impl Eq for PPU {}

/// Contents of the `ppu` save-state section. Excludes `lcd` and `scanline_sprites` (see the
/// `PartialEq` note above) and the DMA controller, which has its own `dma` section.
#[derive(Debug, Clone, Decode, Encode)]
pub struct PpuSection {
    pub vram: [u8; 0x2000],
    pub oam: [u8; 0xA0],
    pub lcd_control: LcdControl,
    pub lcd_status: LcdStatus,
    pub vblank_interrupt_pending: bool,
    pub scroll: Point8,
    pub window_position: Point8,
    pub palette: LcdPalette,
    pub current_ticks: usize,
    pub current_x: usize,
    pub window_state: WindowRenderState,
}

/// Bumped to 2 by B2, which appended VRAM bank 1. The first field keeps its v1 shape — bank 0 —
/// so every fixture written before CGB support still decodes without conversion.
pub const PPU_SECTION_VERSION: u16 = 2;
/// Bumped to 2 by A7: incremental transfer, source page instead of address, `FF46` read-back.
pub const DMA_SECTION_VERSION: u16 = 2;
/// Phase B's own section: everything that only exists on a Game Boy Color.
pub const CGB_SECTION_VERSION: u16 = 1;

/// Contents of the PPU's half of the `cgb` save-state section. The MMU appends its own fields
/// after these — see [`crate::mmu::MMU::write_sections`].
#[derive(Debug, Clone, Decode, Encode)]
pub struct CgbVideoSection {
    pub vram_bank: usize,
    pub background: PaletteBankState,
    pub object: PaletteBankState,
    pub object_priority: u8,
}

impl PPU {
    pub(crate) fn write_sections(&self, writer: &mut SectionWriter) -> Result<(), String> {
        let mut bank_0 = [0u8; VRAM_BANK_SIZE];
        bank_0.copy_from_slice(&self.vram[..VRAM_BANK_SIZE]);
        let mut bank_1 = [0u8; VRAM_BANK_SIZE];
        bank_1.copy_from_slice(&self.vram[VRAM_BANK_SIZE..]);

        writer.write_fields(labels::PPU, PPU_SECTION_VERSION, |fields| {
            fields.field(&PpuSection {
                vram: bank_0,
                oam: self.oam,
                lcd_control: self.lcd_control.clone(),
                lcd_status: self.lcd_status.clone(),
                vblank_interrupt_pending: self.vblank_interrupt_pending,
                scroll: self.scroll,
                window_position: self.window_position,
                palette: self.palette,
                current_ticks: self.current_ticks,
                current_x: self.current_x,
                window_state: self.window_state,
            })?;
            fields.field(&bank_1) // appended in v2
        })?;
        writer.write(labels::DMA, DMA_SECTION_VERSION, &self.dma)
    }

    /// The PPU's contribution to the `cgb` section. Written unconditionally so that a DMG state
    /// and a CGB state have the same shape; on DMG every value here is its default.
    pub(crate) fn write_cgb_fields(&self, fields: &mut crate::savestate::FieldWriter) -> Result<(), String> {
        fields.field(&CgbVideoSection {
            vram_bank: self.vram_bank,
            background: (&self.cgb_background).into(),
            object: (&self.cgb_object).into(),
            object_priority: self.object_priority,
        })
    }

    pub(crate) fn read_cgb_fields(&mut self, fields: &mut crate::savestate::FieldReader) -> Result<(), String> {
        if let Some(section) = fields.field::<CgbVideoSection>()? {
            self.vram_bank = section.vram_bank & (VRAM_BANKS - 1);
            self.cgb_background = section.background.into();
            self.cgb_object = section.object.into();
            self.object_priority = section.object_priority;
        }
        Ok(())
    }

    /// The `dma` section's shape changed in version 2 (A7: incremental transfer, page instead of
    /// address, `FF46` read-back). Version 1 payloads are converted rather than regenerated —
    /// this is the escape hatch documented in `src/savestate/mod.rs` for a shape change.
    fn read_dma_section(&mut self, reader: &SectionReader) -> Result<(), String> {
        let Some(mut fields) = reader.section(labels::DMA)? else {
            return Ok(());
        };
        self.dma = if fields.version() >= 2 {
            match fields.field::<LcdDma>()? {
                Some(dma) => dma,
                None => return Ok(()),
            }
        } else {
            match fields.field::<crate::lcd_dma::LcdDmaV1>()? {
                Some(old) => old.into(),
                None => return Ok(()),
            }
        };
        Ok(())
    }

    pub(crate) fn read_sections(&mut self, reader: &SectionReader) -> Result<(), String> {
        let Some(mut fields) = reader.section(labels::PPU)? else {
            return self.read_dma_section(reader);
        };
        if let Some(section) = fields.field::<PpuSection>()? {
            self.vram[..VRAM_BANK_SIZE].copy_from_slice(&section.vram);
            // Absent in v1 payloads, which is not an error — the fixture predates CGB support and
            // its bank 1 was never anything but zeroes.
            self.vram[VRAM_BANK_SIZE..].copy_from_slice(
                &fields.field::<[u8; VRAM_BANK_SIZE]>()?.unwrap_or([0; VRAM_BANK_SIZE]),
            );
            self.oam = section.oam;
            self.lcd_control = section.lcd_control;
            self.lcd_status = section.lcd_status;
            self.vblank_interrupt_pending = section.vblank_interrupt_pending;
            self.scroll = section.scroll;
            self.window_position = section.window_position;
            self.palette = section.palette;
            self.current_ticks = section.current_ticks;
            self.current_x = section.current_x;
            self.window_state = section.window_state;

            // Derived state: the frame buffer is rebuilt as the PPU walks the rest of the frame,
            // and the sprite list on the next OAM scan.
            self.lcd = [LcdColor::WHITE; LCD_WIDTH * LCD_HEIGHT];
            self.scanline_sprite_count = 0;
        }
        self.read_dma_section(reader)
    }
}

impl Default for PPU {
    fn default() -> Self {
        Self {
            vram: [0; VRAM_BANK_SIZE * VRAM_BANKS],
            vram_bank: 0,
            oam: [0; 0xA0],
            lcd_control: LcdControl::default(),
            lcd_status: LcdStatus::default(),
            vblank_interrupt_pending: false,
            scroll: Point8::default(),
            window_position: Point8::default(),
            palette: LcdPalette::default(),
            cgb_background: PaletteBank::default(),
            cgb_object: PaletteBank::default(),
            object_priority: 0,
            dma: LcdDma::default(),
            lcd: [LcdColor::WHITE; LCD_WIDTH * LCD_HEIGHT],
            current_ticks: 0,
            hblank_started: false,
            color_mode: ColorMode::Dmg,
            current_x: 0,
            window_state: WindowRenderState::default(),
            scanline_sprites: [Sprite::default(); MAX_SPRITES_PER_SCANLINE],
            scanline_sprite_count: 0,
            scanline_sprite_order: [0; MAX_SPRITES_PER_SCANLINE],
            scanline_sprite_columns: [0; LCD_WIDTH.div_ceil(64)],
        }
    }
}

impl PPU {
    pub fn lcd(&self) -> &[LcdColor; LCD_WIDTH * LCD_HEIGHT] {
        &self.lcd
    }

    /// Which palette hardware drives the screen. Set once at construction from the console model
    /// and the cartridge header; see [`ColorMode`].
    pub fn color_mode(&self) -> ColorMode {
        self.color_mode
    }

    pub fn set_color_mode(&mut self, color_mode: ColorMode) {
        self.color_mode = color_mode;
    }

    pub fn read_vram(&self, address: u16) -> u8 {
        if self.lcd_status.mode().vram_accessible() {
            self.vram[self.vram_offset(address)]
        } else {
            // garbage data https://gbdev.io/pandocs/Rendering.html
            0xff
        }
    }

    /// **VRAM bank 0.** The Pokémon layer reads tile data through this and a DMG has no other
    /// bank, so it deliberately does not follow `VBK`. Use [`PPU::vram_banked`] for the bank the
    /// CPU currently sees.
    pub fn vram(&self) -> &[u8] {
        &self.vram[..VRAM_BANK_SIZE]
    }

    pub fn vram_banked(&self, bank: usize) -> &[u8] {
        let base = (bank & (VRAM_BANKS - 1)) * VRAM_BANK_SIZE;
        &self.vram[base..base + VRAM_BANK_SIZE]
    }

    /// `VBK` (`FF4F`) read-back: only bit 0 exists, and the rest read 1.
    pub fn vram_bank_register(&self) -> u8 {
        0xFE | self.vram_bank as u8
    }

    pub fn set_vram_bank_register(&mut self, value: u8) {
        self.vram_bank = (value & 0x01) as usize;
    }

    pub fn cgb_background_palettes(&self) -> &PaletteBank {
        &self.cgb_background
    }

    pub fn cgb_background_palettes_mut(&mut self) -> &mut PaletteBank {
        &mut self.cgb_background
    }

    pub fn cgb_object_palettes(&self) -> &PaletteBank {
        &self.cgb_object
    }

    pub fn cgb_object_palettes_mut(&mut self) -> &mut PaletteBank {
        &mut self.cgb_object
    }

    /// True once per scanline, on entering HBlank. See [`PPU::hblank_started`].
    pub fn consume_hblank_started(&mut self) -> bool {
        std::mem::take(&mut self.hblank_started)
    }

    /// `OPRI` (`FF6C`) read-back: only bit 0 exists.
    pub fn object_priority_register(&self) -> u8 {
        0xFE | self.object_priority
    }

    pub fn set_object_priority_register(&mut self, value: u8) {
        self.object_priority = value & 0x01;
    }

    /// Raw OAM, bypassing the CPU's mode gate. For the DMA controller, debugging and tests.
    pub fn oam(&self) -> &[u8] {
        &self.oam
    }

    pub fn write_vram(&mut self, address: u16, value: u8) {
        if self.lcd_status.mode().vram_accessible() {
            let offset = self.vram_offset(address);
            self.vram[offset] = value;
        }
    }

    /// Where a `0x8000`-relative address lands in the two-bank array, following `VBK`.
    ///
    /// Masked so the index is provably in range: `vram_bank` is a plain `usize`, and without the
    /// mask every VRAM access — the hottest read in the machine — carries a bounds check. Same
    /// reasoning as [`crate::mmu::MMU::work_ram_offset`].
    #[inline]
    fn vram_offset(&self, address: u16) -> usize {
        (self.vram_bank * VRAM_BANK_SIZE + address as usize) & (VRAM_BANK_SIZE * VRAM_BANKS - 1)
    }

    /// Privileged VRAM write for the HDMA/GDMA controller, which owns the bus for the duration of
    /// a block and is not subject to the CPU's mode gate. Like OAM DMA (see
    /// [`PPU::write_oam_dma`]), the transfer targets the bank `VBK` currently selects.
    pub fn write_vram_dma(&mut self, address: u16, value: u8) {
        let offset = self.vram_offset(address & 0x1FFF);
        self.vram[offset] = value;
    }

    /// CPU-facing OAM read. Blocked by the PPU mode, **and** while an OAM DMA holds the bus — the
    /// old code had `|| dma.is_active()`, which made OAM *more* accessible during a transfer,
    /// the opposite of hardware.
    pub fn read_oam(&self, address: u16) -> u8 {
        if self.lcd_status.mode().oam_accessible() && !self.dma.is_active() {
            self.oam[address as usize]
        } else {
            // garbage data https://gbdev.io/pandocs/Rendering.html
            0xff
        }
    }

    /// CPU-facing OAM write. See [`PPU::read_oam`]; use [`PPU::write_oam_dma`] for the transfer
    /// itself.
    pub fn write_oam(&mut self, address: u16, value: u8) {
        if self.lcd_status.mode().oam_accessible() && !self.dma.is_active() {
            self.oam[address as usize] = value;
        }
    }

    /// Privileged OAM write used by the DMA controller, which owns the bus for the duration of a
    /// transfer and is not subject to the CPU's mode gate.
    pub fn write_oam_dma(&mut self, address: u16, value: u8) {
        self.oam[address as usize] = value;
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

    /// Generate a screenshot of the current PPU state as an in-memory RGB image.
    ///
    /// Still an `RgbImage` after B4 widened the frame buffer, so every screenshot test is
    /// unaffected; on DMG the bytes are identical to what they always were.
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
        let mut img = ImageBuffer::new(TILE_MAP_PIXELS as u32, TILE_MAP_PIXELS as u32);
        for y in 0..TILE_MAP_PIXELS {
            for x in 0..TILE_MAP_PIXELS {
                let (color_index, _) = self.map_pixel(tile_map_mode, data_mode, x, y);
                let pixel_color = DMGColor::from_repr(color_index).unwrap_or(DMGColor::White).to_rgb();
                img.put_pixel(x as u32, y as u32, pixel_color);
            }
        }
        img
    }

    /// How many **video** M-cycles until the PPU's next mode transition, or `None` while the LCD
    /// is off.
    ///
    /// This is the bound C2's HALT skip is allowed to jump to. Everything the PPU can do that the
    /// rest of the machine notices — the VBlank and STAT interrupts, `LY`, the HBlank edge that
    /// paces HDMA, the OAM scan — happens *at* a transition, so a halted CPU can sleep right up to
    /// one and observe nothing.
    ///
    /// Never zero: `update` services one transition per call, so a large window can leave
    /// `current_ticks` already past the *next* mode's threshold. Returning 0 there would stall the
    /// run loop; returning 1 lets the PPU walk out over the following steps.
    pub fn next_event(&self) -> Option<u64> {
        if !self.lcd_control.is_enabled() {
            return None;
        }
        let threshold = match self.lcd_status.mode() {
            LcdMode::OAM => OAM_TICKS,
            LcdMode::Drawing => INITIAL_FIFO_LOAD_TICKS + LCD_WIDTH,
            LcdMode::HBlank => SCANLINE_TICKS - OAM_TICKS - INITIAL_FIFO_LOAD_TICKS - LCD_WIDTH,
            LcdMode::VBlank => SCANLINE_TICKS,
        };
        let remaining_t_cycles = threshold.saturating_sub(self.current_ticks);
        Some((remaining_t_cycles as u64).div_ceil(4).max(1))
    }

    pub fn update(&mut self, delta_machine_cycles: MachineCycles) {
        if !self.lcd_control.is_enabled() {
            // TODO should the screen be blanked?
            return
        }

        self.current_ticks += delta_machine_cycles.t_cycles() as usize; // TODO the PPU is twice as slow in CGB double speed mode

        match self.lcd_status.mode() {
            LcdMode::OAM => {
                if self.current_ticks >= OAM_TICKS {
                    self.lcd_status.set_mode(LcdMode::Drawing);
                    self.current_ticks -= OAM_TICKS;

                    self.scan_oam();
                }
            }
            LcdMode::Drawing => {
                let drawing_ticks = INITIAL_FIFO_LOAD_TICKS + LCD_WIDTH;

                if self.current_ticks >= drawing_ticks {
                    // Flush whatever is still outstanding before leaving mode 3. Previously this
                    // branch drew nothing at all — masked only by the x-advance bug below, which
                    // had already emitted every pixel far too early.
                    self.draw_pixels_to(LCD_WIDTH);
                    self.lcd_status.set_mode(LcdMode::HBlank); // drawing done
                    self.hblank_started = true; // paces HDMA — see MMU::update
                    self.current_ticks -= drawing_ticks;
                } else {
                    // `current_ticks` is an *absolute* offset into mode 3 and is not reset within
                    // it, so the old `start_x + current_ticks - INITIAL_FIFO_LOAD_TICKS + 1` added
                    // the offset afresh every call and advanced x quadratically: 1, 6, 15, 28, 45,
                    // ... all 160 pixels were emitted ~36 T into mode 3 instead of over 160 T.
                    // Nothing looked broken because an `x < LCD_WIDTH` guard swallowed the
                    // overshoot — but every register write landing more than ~36 cycles into
                    // mode 3 was a no-op for that scanline.
                    let target_x = self.current_ticks
                        .saturating_sub(INITIAL_FIFO_LOAD_TICKS)
                        .min(LCD_WIDTH);
                    self.draw_pixels_to(target_x);
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
        self.banked_tile(mode, index, 0)
    }

    fn banked_tile(&self, mode: TileDataMode, index: u8, bank: usize) -> Tile {
        // Masked for the same reason as `vram_offset`: a slice whose bounds the compiler cannot
        // prove costs a check per pixel here.
        let address = (bank * VRAM_BANK_SIZE + mode.tile_address(index) as usize - VRAM_BASE_ADDRESS)
            & (VRAM_BANK_SIZE * VRAM_BANKS - 1);
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
    ///
    /// LCDC bit 0 gates the window on DMG but not on CGB, where it means something else entirely
    /// — see [`LcdControl::background_enabled`].
    fn in_window(&self, x: usize, _y: usize) -> bool {
        self.window_enabled()
            && self.window_state.is_active
            && x >= self.window_position.x.saturating_sub(7) as usize
    }

    fn window_pixel(&self, x: usize) -> (u8, TileAttributes) {
        self.map_pixel(
            self.lcd_control.window_tile_map(),
            self.lcd_control.tile_data_mode(),
            // x+7 because window starts at x position - 7
            x + 7 - self.window_position.x as usize,
            // the y coordinate is derived from the total number of window lines rendered
            self.window_state.window_y
        )
    }

    fn bg_pixel(&self, x: usize, y: usize) -> (u8, TileAttributes) {
        self.map_pixel(
            self.lcd_control.background_tile_map(),
            self.lcd_control.tile_data_mode(),
            (x as u8).wrapping_add(self.scroll.x) as usize,
            (y as u8).wrapping_add(self.scroll.y) as usize
        )
    }

    /// LCDC's window-enable bit — which is a *different* bit on CGB. Split out of [`PPU::in_window`]
    /// so the pixel loop can settle it once instead of once per pixel.
    fn window_enabled(&self) -> bool {
        if self.color_mode.cgb_features() {
            self.lcd_control.window_display_enabled()
        } else {
            self.lcd_control.window_enabled()
        }
    }

    /// Fetch the two bitplane bytes of the tile row covering map-space row `map_y`, for the tile at
    /// tile-map index `entry` — the per-*tile* half of what [`PPU::map_pixel`] does per pixel.
    ///
    /// Same reads, same order, same masking as `map_pixel`; only the granularity differs.
    #[inline]
    fn fetch_tile_row(&self, data_mode: TileDataMode, entry: usize, map_y: usize) -> TileRow {
        let index = self.vram[entry];
        let attributes = if self.color_mode.cgb_features() {
            TileAttributes(self.vram[VRAM_BANK_SIZE + entry])
        } else {
            TileAttributes::NONE
        };

        let mut pixel_y = map_y % TILE_PIXELS;
        if attributes.flip_y() {
            pixel_y = TILE_PIXELS - 1 - pixel_y;
        }
        // Masked for the same reason as `banked_tile`: a bound the compiler cannot prove costs a
        // check on the hottest path in the emulator.
        let base = (attributes.bank() * VRAM_BANK_SIZE + data_mode.tile_address(index) as usize
            - VRAM_BASE_ADDRESS)
            & (VRAM_BANK_SIZE * VRAM_BANKS - 1);
        let row = base + pixel_y * 2;
        TileRow {
            entry,
            lo: self.vram[row],
            hi: self.vram[row + 1],
            attributes,
            colors: std::array::from_fn(|index| self.background_color(index as u8, attributes)),
        }
    }

    /// One pixel of a background or window tile map, in 256x256 map space.
    ///
    /// On CGB the tile map at `0x9800`/`0x9C00` is shadowed at the same offset in **VRAM bank 1**
    /// by a byte of per-tile attributes (gambatte `video/ppu.cpp:617`): palette, tile bank, X and
    /// Y flip, and BG-over-OBJ priority. On DMG, and in CGB compatibility mode, no such byte
    /// exists and the attributes are all-zero — which happens to be exactly "palette 0, bank 0,
    /// no flips, no priority", so the two paths converge without a branch per pixel.
    fn map_pixel(&self, map: TileMapMode, data_mode: TileDataMode, x: usize, y: usize) -> (u8, TileAttributes) {
        let entry = map.base_address() as usize - VRAM_BASE_ADDRESS
            + (y / TILE_PIXELS % TILE_MAP_SIZE) * TILE_MAP_SIZE
            + (x / TILE_PIXELS % TILE_MAP_SIZE);
        let index = self.vram[entry];
        let attributes = if self.color_mode.cgb_features() {
            TileAttributes(self.vram[VRAM_BANK_SIZE + entry])
        } else {
            TileAttributes::NONE
        };

        let mut pixel_x = x % TILE_PIXELS;
        let mut pixel_y = y % TILE_PIXELS;
        if attributes.flip_x() { pixel_x = TILE_PIXELS - 1 - pixel_x; }
        if attributes.flip_y() { pixel_y = TILE_PIXELS - 1 - pixel_y; }

        let color = self.banked_tile(data_mode, index, attributes.bank()).pixel(pixel_x, pixel_y);
        (color, attributes)
    }

    pub fn tile_indexes_of_vram_addresses(&self, address: u16, length: usize) -> Vec<u8> {
        debug_assert!(
            TileDataMode::is_valid_tile_address(address),
            "Tile addresses must be in the range 0x8000-0x9FFF"
        );
        debug_assert!(length % TILE_BYTES == 0, "Length must be a multiple of 16 bytes");

        let mode = self.lcd_control.tile_data_mode();
        let mut indexes = Vec::with_capacity(length / TILE_BYTES);

        for offset in (0..length).step_by(TILE_BYTES) {
            if let Some(index) = mode.tile_index(address + offset as u16) {
                indexes.push(index);
            }
        }
        indexes
    }

    /// Where each of `tile_indexes` appears **on the 20x18 screen**, in screen tile coordinates.
    ///
    /// Used by the Pokémon layer to read the game's own text out of the tile map — see
    /// [`crate::pokemon::PokemonApiTrait::on_screen_text`].
    ///
    /// ⚠️ **Only what is actually displayed, and this used to scan all 32x32 of both maps at raw
    /// map coordinates.** A tile map is 256x256 pixels and the screen shows 160x144 of it, so most
    /// of both maps is off-screen at any moment — and Pokémon Red leaves things there. During a
    /// battle a stale copy of the enemy's HUD sits below the visible rows of the window map, so
    /// every read of the message box came back as
    /// `"GEODUDE BRN Ember 23 22/ 63 Enemy GEODUDE's hurt by the burn! GEODUDE 10"`: the message,
    /// with an invisible `GEODUDE 10` welded onto the end of it. Sorted by position that lands
    /// *after* the message, so every frame of a page ended with the same nine characters, no frame
    /// was a prefix of the next, and [`crate::pokemon::text::PokemonTextReader`] broke the page up
    /// into one fragment per frame — 1456 bytes of `GEODUDE 10 Ene GEODUDE 10 Enem …` in a single
    /// text box, on the deployed run of 2026-08-27.
    ///
    /// ⚠️ **The window is asked about per screen tile, not once for the frame.** The old version
    /// took "the window is enabled" to mean "the window is everything", so anything in the window
    /// map won over the background wherever it was. WX and WY decide that, and Pokémon Red parks
    /// the window at WY=144 — entirely off-screen — for the whole of the overworld while its map
    /// still holds the last screen drawn through it.
    pub fn tile_coordinates(&self, tile_indexes: &[u8]) -> Vec<(usize, Point8)> {
        let bg_tile_map = self.tile_map(self.lcd_control.background_tile_map());
        let window_tile_map = self.tile_map(self.lcd_control.window_tile_map());

        // map of tile indexes to their position in the tile_indexes array
        let tile_lookups: HashMap<u8, usize> = tile_indexes.iter().enumerate()
            .map(|(i, &index)| (index, i))
            .collect();

        // ⚠️ **[`Self::window_enabled`], not `LcdControl`'s.** The window-enable bit is gated by
        // LCDC bit 0 on DMG and means something else entirely on CGB, and this has to agree with
        // the pixel loop about which surface is on screen.
        let window_enabled = self.window_enabled();
        let (wx, wy) = (self.window_position.x as usize, self.window_position.y as usize);

        let mut coordinates = Vec::new();
        for y in 0..LCD_HEIGHT / TILE_PIXELS {
            for x in 0..LCD_WIDTH / TILE_PIXELS {
                let (px, py) = (x * TILE_PIXELS, y * TILE_PIXELS);
                // The same test the pixel loop makes — see [`Self::in_window`]. `window_state` is
                // per-scanline and only meaningful mid-frame, so the static half of it is used
                // here: the window covers this tile if it starts at or above this row and at or
                // left of this column.
                let index = match window_enabled && py >= wy && px + 7 >= wx {
                    true => {
                        let (wtx, wty) = ((px + 7 - wx) / TILE_PIXELS, (py - wy) / TILE_PIXELS);
                        match wtx < TILE_MAP_SIZE && wty < TILE_MAP_SIZE {
                            true => window_tile_map.tile_index(wtx, wty),
                            false => continue,
                        }
                    }
                    false => bg_tile_map.tile_index(
                        (px as u8).wrapping_add(self.scroll.x) as usize / TILE_PIXELS,
                        (py as u8).wrapping_add(self.scroll.y) as usize / TILE_PIXELS,
                    ),
                };
                if let Some(&found) = tile_lookups.get(&index) {
                    coordinates.push((found, Point8 { x: x as u8, y: y as u8 }));
                }
            }
        }

        coordinates
    }


    /// Emit pixels for the current scanline up to (but excluding) `target_x`, then leave
    /// `current_x` there. A no-op if the pixel clock has not moved.
    ///
    /// **Deliberately not inlined.** `PPU::update` is inlined into `MMU::update`, which the CPU
    /// calls once per instruction; letting the whole pixel path in there grew that function by
    /// 60% and cost several percent of core throughput to instruction-cache pressure alone.
    #[inline(never)]
    fn draw_pixels_to(&mut self, target_x: usize) {
        let y = self.lcd_status.ly() as usize;

        if self.lcd_status.ly() == self.window_position.y && !self.window_state.is_active {
            self.window_state.activate(y, self.window_position);
        }

        if target_x <= self.current_x {
            return;
        }

        // On CGB, LCDC bit 0 does not switch the background off — it drops its *priority*.
        let cgb = self.color_mode.cgb_features();
        let bg_drawn = cgb || self.lcd_control.background_enabled();
        let bg_has_priority = self.lcd_control.background_enabled();

        // ⭐ **Everything from here to the loop is hoisted out of it, and that is the whole point.**
        // Measured by `perf`, this loop was 36% of the emulator, and its cost was a chain of
        // dependent loads *per pixel*: tile-map entry → tile row → palette → colour. Eight
        // consecutive pixels share all of it.
        //
        // Nothing here can change while the loop runs: the CPU is between memory accesses for the
        // whole of one `draw_pixels_to`, so VRAM, LCDC, SCX/SCY and WX are fixed. The cached fetch
        // is therefore **exactly** what refetching would have produced — which is why dmg-acid2 and
        // cgb-acid2 still match their reference images byte for byte.
        let bg_map = self.lcd_control.background_tile_map();
        let window_map = self.lcd_control.window_tile_map();
        let data_mode = self.lcd_control.tile_data_mode();
        // `in_window` reduced to a comparison: its other two terms are loop-invariant, and
        // `is_active` in particular is settled by the `activate` call above.
        let window_from_x = if self.window_enabled() && self.window_state.is_active {
            self.window_position.x.saturating_sub(7) as usize
        } else {
            usize::MAX
        };

        // What a background-disabled pixel resolves to. Also a loop constant.
        let blank_color = self.background_color(0, TileAttributes::NONE);
        let mut bg_row = TileRow::EMPTY;
        let mut window_row = TileRow::EMPTY;
        let mut row_in_window = false;
        for x in self.current_x..target_x {
            let pixel_in_window = x >= window_from_x;
            if pixel_in_window && !row_in_window {
                row_in_window = true;
                self.window_state.update_if_active(y);
                // ⚠️ That call can move `window_y`, so anything fetched against the old one is
                // stale. It happens at most once per call, on the first window pixel.
                window_row = TileRow::EMPTY;
            }

            let mut bg_color = blank_color;
            let (bg_color_index, bg_attributes) = if !bg_drawn {
                (0, TileAttributes::NONE)
            } else {
                let (map, map_x, map_y, cached) = if pixel_in_window {
                    // x+7 because the window starts at WX-7.
                    (window_map, x + 7 - self.window_position.x as usize,
                     self.window_state.window_y, &mut window_row)
                } else {
                    (bg_map, (x as u8).wrapping_add(self.scroll.x) as usize,
                     (y as u8).wrapping_add(self.scroll.y) as usize, &mut bg_row)
                };
                let entry = tile_map_entry(map, map_x, map_y);
                if cached.entry != entry {
                    *cached = self.fetch_tile_row(data_mode, entry, map_y);
                }
                let index = cached.pixel(map_x % TILE_PIXELS);
                bg_color = cached.colors[index as usize];
                (index, cached.attributes)
            };

            let color = self.top_sprite(x, y)
                .map_or(bg_color, |(sprite, sprite_color)| {
                    // The background wins when it is opaque *and* either the sprite yields to it
                    // or (CGB only) the tile claims priority. LCDC bit 0 overrides both on CGB.
                    let bg_wins = bg_color_index != 0
                        && (sprite.bg_priority || bg_attributes.priority())
                        && (!cgb || bg_has_priority);
                    if bg_wins {
                        bg_color
                    } else {
                        self.sprite_color(sprite, sprite_color)
                    }
                });

            self.lcd[y * LCD_WIDTH + x] = color;
        }

        self.current_x = target_x;
    }

    /// The highest-priority non-transparent sprite covering `x`, if any.
    ///
    /// DMG breaks ties by X coordinate, then by OAM index. CGB uses OAM index alone — the order
    /// `scanline_sprites` is already in — unless `OPRI` bit 0 asks for the DMG rule, which is
    /// what the boot ROM does in compatibility mode (gambatte `video/ppu.cpp:853-884`).
    fn top_sprite(&self, x: usize, y: usize) -> Option<(&Sprite, u8)> {
        // No sprite is over this column: the overwhelmingly common case on most scanlines, and the
        // scan below would have to walk every selected sprite to discover it.
        if self.scanline_sprite_columns[x / 64] & (1 << (x % 64)) == 0 {
            return None;
        }
        // Whichever order applies, the winner is the *first* candidate in it — so this is a scan
        // that stops early, not a sort. C5: it used to build and sort a `Vec` per pixel.
        let oam_order = self.color_mode.cgb_features() && self.object_priority & 0x01 == 0;
        for i in 0..self.scanline_sprite_count {
            let index = if oam_order { i } else { self.scanline_sprite_order[i] as usize };
            let sprite = &self.scanline_sprites[index];
            if sprite.x > x as isize || sprite.x + TILE_PIXELS as isize <= x as isize {
                continue;
            }
            let sprite_color = self.sprite_pixel(sprite, x, y);
            if sprite_color != 0 { // transparent pixels do not compete
                return Some((sprite, sprite_color));
            }
        }
        None
    }

    fn background_color(&self, color_index: u8, attributes: TileAttributes) -> LcdColor {
        match self.color_mode {
            ColorMode::Dmg => self.palette.background()[color_index as usize].to_lcd(),
            // Compatibility mode still runs the index through `BGP`; the shade that comes out is
            // then a *palette index* into CGB BG palette 0.
            ColorMode::CgbCompat => {
                let shade = self.palette.background()[color_index as usize] as u8;
                self.cgb_background.color(0, shade)
            }
            ColorMode::Cgb => self.cgb_background.color(attributes.palette(), color_index),
        }
    }

    fn sprite_color(&self, sprite: &Sprite, color_index: u8) -> LcdColor {
        match self.color_mode {
            ColorMode::Dmg => sprite.palette(&self.palette)[color_index as usize].to_lcd(),
            ColorMode::CgbCompat => {
                let shade = sprite.palette(&self.palette)[color_index as usize] as u8;
                self.cgb_object.color(sprite.alt_palette as u8, shade)
            }
            ColorMode::Cgb => self.cgb_object.color(sprite.cgb_palette, color_index),
        }
    }

    fn sprite_pixel(&self, sprite: &Sprite, x: usize, y: usize) -> u8 {
        // Use the height this sprite was *selected* under, not the current LCDC — see `Sprite`.
        let sprite_x = (x as isize - sprite.x) as usize;
        let pixel_x = if sprite.flip_x { TILE_PIXELS - 1 - sprite_x } else { sprite_x };
        let sprite_y = (y as isize - sprite.y) as usize;
        let pixel_y = if sprite.flip_y { sprite.height - 1 - sprite_y } else { sprite_y };
        let bank = if self.color_mode.cgb_features() { sprite.vram_bank as usize } else { 0 };

        if sprite.height <= TILE_PIXELS {
            self.banked_tile(TileDataMode::Lower, sprite.tile_index, bank).pixel(pixel_x, pixel_y)
        } else if pixel_y < TILE_PIXELS {
            self.banked_tile(TileDataMode::Lower, sprite.tile_index & 0xFE, bank)
                .pixel(pixel_x, pixel_y)
        } else {
            self.banked_tile(TileDataMode::Lower, sprite.tile_index | 0x01, bank)
                .pixel(pixel_x, pixel_y - TILE_PIXELS)
        }
    }

    /// The mode 2 → mode 3 OAM scan: pick the sprites covering this scanline.
    ///
    /// ⚠️ **The semantics here are already correct and easy to break** — see
    /// `docs/compatibility/03-ppu.md`, "Sprites — what is already correct". The ten-per-line limit
    /// is taken **in OAM order after a Y-only filter** (an eleventh sprite is dropped even if it
    /// would have won on X), and DMG's X tie-break must be **stable**, so equal X keeps OAM order.
    ///
    /// C5 rewrote it to fill fixed arrays instead of collecting two `Vec`s per scanline — one of
    /// 40 sprites and one of up to 10 — and to derive the X order once here rather than
    /// re-deriving it for every one of the 160 pixels.
    fn scan_oam(&mut self) {
        self.scanline_sprite_count = 0;
        self.scanline_sprite_columns = [0; LCD_WIDTH.div_ceil(64)];
        if !self.lcd_control.objects_enabled() {
            return;
        }

        let y = self.lcd_status.ly() as isize;
        let height = self.lcd_control.object_size().height();
        for i in 0..SPRITE_COUNT {
            let start = i * SPRITE_BYTES;
            let sprite = Sprite::new(&self.oam[start..start + SPRITE_BYTES], height);
            if y < sprite.y || y >= sprite.y + height as isize {
                continue;
            }
            self.scanline_sprites[self.scanline_sprite_count] = sprite;
            self.scanline_sprite_count += 1;
            if self.scanline_sprite_count == MAX_SPRITES_PER_SCANLINE {
                break;
            }
        }

        // Which columns any of them touches, so the pixel loop can skip the scan entirely.
        for i in 0..self.scanline_sprite_count {
            let start = self.scanline_sprites[i].x.max(0) as usize;
            let end = (self.scanline_sprites[i].x + TILE_PIXELS as isize).clamp(0, LCD_WIDTH as isize) as usize;
            for column in start..end {
                self.scanline_sprite_columns[column / 64] |= 1 << (column % 64);
            }
        }

        // Insertion sort — stable, and the right choice at n <= 10 (gambatte's `sprite_mapper.cpp`
        // uses one too). Only the *indices* move, so `scanline_sprites` keeps OAM order for CGB.
        for i in 0..self.scanline_sprite_count {
            let mut j = i;
            let x = self.scanline_sprites[i].x;
            while j > 0 && self.scanline_sprites[self.scanline_sprite_order[j - 1] as usize].x > x {
                self.scanline_sprite_order[j] = self.scanline_sprite_order[j - 1];
                j -= 1;
            }
            self.scanline_sprite_order[j] = i as u8;
        }
    }
}


const VRAM_BASE_ADDRESS: usize = 0x8000;
pub const VRAM_BANK_SIZE: usize = 0x2000;
/// Both banks are allocated even on DMG, as gambatte does — it keeps the save-state shape and
/// every index calculation identical across models, and 8 KB is not worth a branch.
pub const VRAM_BANKS: usize = 2;
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

impl Activation for PPU {
    fn is_activation_pending(&self) -> bool {
        self.vblank_interrupt_pending
    }

    fn clear_activation(&mut self) {
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

/// Where a tile-map lookup lands in VRAM, for map-space pixel `(x, y)`.
///
/// Split out of [`PPU::map_pixel`] so the pixel loop can ask "same tile as last pixel?" without
/// touching memory — the comparison is pure arithmetic, and it is what turns a per-pixel fetch
/// into a per-tile one.
#[inline]
fn tile_map_entry(map: TileMapMode, x: usize, y: usize) -> usize {
    map.base_address() as usize - VRAM_BASE_ADDRESS
        + (y / TILE_PIXELS % TILE_MAP_SIZE) * TILE_MAP_SIZE
        + (x / TILE_PIXELS % TILE_MAP_SIZE)
}

/// One background or window tile row, fetched once and shifted out over the (up to eight) pixels
/// that share it.
///
/// This is the shape gambatte's PPU has always had — fetch a row, shift it out — and the reason
/// its renderer is so much cheaper than a per-pixel `map_pixel`.
#[derive(Debug, Clone, Copy)]
struct TileRow {
    /// The tile-map index this was fetched from; the cache key. [`usize::MAX`] means empty, which
    /// no real entry can be.
    entry: usize,
    lo: u8,
    hi: u8,
    attributes: TileAttributes,
    /// The four colours this tile's two-bit indices resolve to, resolved **once per tile**.
    ///
    /// ⭐ `perf` put **33% of the whole pixel loop** in the dependent chain this replaces: a byte
    /// load from the `BGP` mapping followed by the shade→RGB multiply, per pixel, for a value with
    /// four possible answers. On DMG and in compatibility mode it does not even depend on the tile;
    /// on CGB it depends only on the tile's palette bits, which arrive with the fetch.
    colors: [LcdColor; 4],
}

impl TileRow {
    const EMPTY: Self = Self {
        entry: usize::MAX,
        lo: 0,
        hi: 0,
        attributes: TileAttributes::NONE,
        colors: [LcdColor::WHITE; 4],
    };

    /// The 2-bit colour index at `pixel_x` (0..8) across the row, honouring the CGB X flip — which
    /// is a choice of bit, exactly as flipping the coordinate before indexing was.
    #[inline]
    fn pixel(&self, pixel_x: usize) -> u8 {
        let bit = if self.attributes.flip_x() { pixel_x } else { TILE_PIXELS - 1 - pixel_x };
        ((self.lo >> bit) & 1) | (((self.hi >> bit) & 1) << 1)
    }
}

/// A background or window tile's CGB attribute byte, from VRAM bank 1.
///
/// [`TileAttributes::NONE`] is what DMG and CGB compatibility mode use — no attribute byte exists
/// there, and all-zero is the right answer for every field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct TileAttributes(u8);

impl TileAttributes {
    const NONE: Self = Self(0);

    /// Bits 0-2: which of the eight CGB background palettes.
    #[inline]
    fn palette(self) -> u8 { self.0 & 0x07 }
    /// Bit 3: which VRAM bank holds the tile's pixels.
    #[inline]
    fn bank(self) -> usize { ((self.0 >> 3) & 0x01) as usize }
    /// Bit 5.
    #[inline]
    fn flip_x(self) -> bool { self.0 & 0x20 != 0 }
    /// Bit 6.
    #[inline]
    fn flip_y(self) -> bool { self.0 & 0x40 != 0 }
    /// Bit 7: this tile is drawn over sprites, unless LCDC bit 0 says otherwise.
    #[inline]
    fn priority(self) -> bool { self.0 & 0x80 != 0 }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Decode, Encode)]
struct Sprite {
    y: isize,
    x: isize,
    /// Height in pixels, **captured during the OAM scan that selected this sprite**. LCDC bit 2
    /// is re-readable by the guest mid-scanline, so re-deriving the height at draw time can
    /// disagree with the height the sprite was selected under — an 8x16 sprite selected on rows
    /// 8..15 and then drawn as 8x8 indexes past its 16-byte tile. See A6.
    height: usize,
    tile_index: u8,
    bg_priority: bool, // bit 7 - 0 = No, 1 = BG and Window color indices 1–3 are drawn over this OBJ
    flip_y: bool, // bit 6 - 0 = Normal, 1 = Entire OBJ is vertically mirrored
    flip_x: bool, // bit 5 - 0 = Normal, 1 = Entire OBJ is horizontally mirrored
    alt_palette: bool, // bit 4 (DMG only) - 0 = Use OBJ palette 0, 1 = Use OBJ palette 1
    /// Bit 3 (CGB only): which VRAM bank holds the sprite's tile. A `u8` rather than a `usize`
    /// because `scanline_sprites` is walked once per pixel — the record's size is hot.
    vram_bank: u8,
    /// Bits 0-2 (CGB only): which of the eight CGB object palettes.
    cgb_palette: u8,
}

impl Sprite {
    pub fn new(data: &[u8], height: usize) -> Self {
        debug_assert!(data.len() == SPRITE_BYTES, "Sprite data must be exactly 4 bytes");
        Self {
            y: data[0] as isize - 16, // Y coordinate is offset by 16 pixels
            x: data[1] as isize - 8, // X coordinate is offset by 8 pixels
            height,
            tile_index: data[2],
            bg_priority: (data[3] & 0x80) != 0,
            flip_y: (data[3] & 0x40) != 0,
            flip_x: (data[3] & 0x20) != 0,
            alt_palette: (data[3] & 0x10) != 0,
            vram_bank: (data[3] >> 3) & 0x01,
            cgb_palette: data[3] & 0x07,
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

    /// A11: `current_ticks` is an absolute offset into mode 3, but the old x-advance re-added it
    /// on every call, so `current_x` went 1, 6, 15, 28, 45, 66, 91, 120, 153, 190 — every pixel
    /// emitted ~36 T in, instead of spread across 160. The frame looked fine (an `x < LCD_WIDTH`
    /// guard swallowed the overshoot) but any register write past ~36 T into mode 3 was ignored
    /// for that scanline.
    #[test]
    fn pixel_clock_advances_linearly_through_mode_3() {
        let mut ppu = PPU::default();
        ppu.lcd_control.set(0b1000_0001); // LCD on, background on
        ppu.lcd_status.set_mode(LcdMode::Drawing);

        let mut observed = Vec::new();
        for _ in 0..10 {
            ppu.update(MachineCycles::ONE); // 4 T
            observed.push(ppu.current_x);
        }

        // 4 T per step, minus the 12 T FIFO warm-up: 0, 0, 0, 4, 8, 12, ...
        let expected: Vec<usize> = (1..=10)
            .map(|step| (step * 4usize).saturating_sub(INITIAL_FIFO_LOAD_TICKS).min(LCD_WIDTH))
            .collect();
        assert_eq!(observed, expected);
    }

    /// Leaving mode 3 must flush the rest of the scanline. That branch used to draw nothing at
    /// all — harmless only because the quadratic advance had already over-drawn everything.
    #[test]
    fn leaving_mode_3_flushes_the_rest_of_the_scanline() {
        let mut ppu = PPU::default();
        ppu.lcd_control.set(0b1000_0001);
        ppu.lcd_status.set_mode(LcdMode::Drawing);

        while ppu.lcd_status.mode() == LcdMode::Drawing {
            ppu.update(MachineCycles::ONE);
        }
        assert_eq!(ppu.lcd_status.mode(), LcdMode::HBlank);
        // current_x is reset at the end of HBlank, not on leaving mode 3, so the whole scanline
        // must have been emitted by the time the mode changed.
        assert_eq!(ppu.current_x, LCD_WIDTH);
    }

    /// A6: `scanline_sprites` is filtered at OAM-scan time using the *then* object size, but
    /// `sprite_pixel` used to re-read LCDC at draw time. A guest write between the two flips
    /// 8x16 -> 8x8, so `sprite_y` reaches 8..15 against a 16-byte tile: index up to 31 on a
    /// 16-element slice, which is an out-of-bounds panic in release (the only guard was a
    /// `debug_assert!`). Flipped sprites underflowed `8 - 1 - 15` on a `usize` instead.
    #[test]
    fn lcdc_flip_mid_scanline_does_not_read_past_a_sprite_tile() {
        for flip_y in [false, true] {
            let mut ppu = PPU::default();
            // LCD on, objects on, 8x16.
            ppu.lcd_control.set(0b1000_0110);

            // Place the sprite so that scanline 0 lands 12 rows down it — a row that only exists
            // while objects are 8x16.
            ppu.oam[0] = 4; // y = -12
            ppu.oam[1] = 8; // x = 0
            ppu.oam[2] = 0x02; // tile
            ppu.oam[3] = if flip_y { 0x40 } else { 0x00 };

            // Scan OAM for this scanline.
            ppu.lcd_status.set_mode(LcdMode::OAM);
            ppu.update(MachineCycles::from_t(OAM_TICKS as u64));
            assert_eq!(ppu.scanline_sprite_count, 1, "the sprite should have been selected");
            assert_eq!(ppu.scanline_sprites[0].height, 16, "selected as 8x16");

            // The guest now shrinks objects to 8x8, mid-scanline.
            ppu.lcd_control.set(0b1000_0010);
            assert_eq!(ppu.lcd_control.object_size(), ObjectSizeMode::Single);

            // Step through mode 3 rather than jumping it in one update: a single large update
            // takes the `>= drawing_ticks` branch and never draws a pixel at all.
            assert_eq!(ppu.lcd_status.mode(), LcdMode::Drawing);
            while ppu.lcd_status.mode() == LcdMode::Drawing {
                ppu.update(MachineCycles::ONE); // must not panic
            }
            assert!(ppu.current_x > 0, "the scanline must actually have been drawn");
        }
    }

    /// Phase B's pixel path, at the level `cgb-acid2` cannot reach: that ROM only ever uses an
    /// identity `BGP`, so it says nothing about whether compatibility mode really indirects
    /// through the DMG palette registers. (Verified by mutation: dropping that indirection left
    /// every other Phase B test green.)
    mod cgb {
        use super::*;
        use crate::lcd_palette::LcdColor;

        /// Four distinguishable RGB555 colours, so a mix-up cannot go unnoticed.
        const PALETTE: [u16; 4] = [0x7C1F, 0x03E0, 0x001F, 0x7FE0];

        /// A PPU showing tile 0 at the top-left of the background, whose first row is the colour
        /// indexes `0, 1, 2, 3, 0, 1, 2, 3`.
        fn ppu_with_tile(color_mode: ColorMode) -> PPU {
            let mut ppu = PPU::default();
            ppu.set_color_mode(color_mode);
            ppu.lcd_control.set(0b1001_0001); // LCD on, tile data at 0x8000, BG on
            ppu.vram[0] = 0b0101_0101; // low bits
            ppu.vram[1] = 0b0011_0011; // high bits
            ppu
        }

        fn set_palette(ppu: &mut PPU, bank: usize, palette: usize, colors: [u16; 4]) {
            let bytes = std::array::from_fn(|i| {
                let color = colors[i / 2];
                if i % 2 == 0 { color as u8 } else { (color >> 8) as u8 }
            });
            if bank == 0 {
                ppu.cgb_background.set_palette(palette, bytes);
            } else {
                ppu.cgb_object.set_palette(palette, bytes);
            }
        }

        /// Render scanline 0 and return its first eight pixels.
        fn scanline(ppu: &mut PPU) -> Vec<LcdColor> {
            ppu.lcd_status.set_mode(LcdMode::OAM);
            while ppu.lcd_status.mode() != LcdMode::HBlank {
                ppu.update(MachineCycles::ONE);
            }
            ppu.lcd[..8].to_vec()
        }

        fn expected(colors: [u16; 4], order: [usize; 4]) -> Vec<LcdColor> {
            (0..8).map(|x| LcdColor::from_rgb555(colors[order[x % 4]])).collect()
        }

        /// ⭐ In compatibility mode the colour index goes through `BGP` **first**, and the shade
        /// that comes out is what indexes CGB palette 0. Skip that step and a game that inverts
        /// `BGP` — which Pokémon Red does, on every screen fade — renders inside out.
        #[test]
        fn compatibility_mode_indirects_the_background_through_bgp() {
            let mut ppu = ppu_with_tile(ColorMode::CgbCompat);
            set_palette(&mut ppu, 0, 0, PALETTE);
            ppu.palette.background_mut().set_from_byte(0b00_01_10_11); // reverse the shades

            assert_eq!(scanline(&mut ppu), expected(PALETTE, [3, 2, 1, 0]));
        }

        /// The same for sprites, through `OBP0`/`OBP1` and into CGB object palette 0 or 1
        /// according to the OAM bit that means "alternate palette" on DMG.
        #[test]
        fn compatibility_mode_indirects_sprites_through_obp() {
            for (alt, cgb_palette) in [(false, 0usize), (true, 1)] {
                let mut ppu = ppu_with_tile(ColorMode::CgbCompat);
                ppu.lcd_control.set(0b1001_0011); // ...and objects on
                set_palette(&mut ppu, 0, 0, [0x0000; 4]);      // background: black, so it cannot be mistaken
                set_palette(&mut ppu, 1, cgb_palette, PALETTE);
                ppu.palette.object0_mut().set_from_byte(0b00_01_10_11);
                ppu.palette.object1_mut().set_from_byte(0b00_01_10_11);

                ppu.oam[0] = 16; // y = 0
                ppu.oam[1] = 8;  // x = 0
                ppu.oam[2] = 0;  // tile 0
                ppu.oam[3] = if alt { 0x10 } else { 0x00 };

                // Index 0 is transparent for a sprite, so the background shows through there.
                let mut want = expected(PALETTE, [3, 2, 1, 0]);
                want[0] = LcdColor::from_rgb555(0x0000);
                want[4] = LcdColor::from_rgb555(0x0000);
                assert_eq!(scanline(&mut ppu), want, "alt_palette = {alt}");
            }
        }

        /// A real CGB game does not go through `BGP` at all: the colour index addresses palette
        /// RAM directly. Setting `BGP` to something perverse must change nothing.
        #[test]
        fn cgb_mode_ignores_the_dmg_palette_registers() {
            let mut ppu = ppu_with_tile(ColorMode::Cgb);
            set_palette(&mut ppu, 0, 0, PALETTE);
            ppu.palette.background_mut().set_from_byte(0b00_01_10_11);

            assert_eq!(scanline(&mut ppu), expected(PALETTE, [0, 1, 2, 3]));
        }

        /// B6. The attribute byte in VRAM bank 1 picks the palette, and X-flips the tile.
        #[test]
        fn bg_attributes_select_the_palette_and_flip_the_tile() {
            let mut ppu = ppu_with_tile(ColorMode::Cgb);
            set_palette(&mut ppu, 0, 5, PALETTE);
            // Tile-map entry 0 lives at 0x9800 = VRAM offset 0x1800; its attribute byte is at the
            // same offset in bank 1.
            ppu.vram[VRAM_BANK_SIZE + 0x1800] = 0x05 | 0x20; // palette 5, X flip

            // Flipped, the row reads 3, 2, 1, 0, 3, 2, 1, 0.
            assert_eq!(scanline(&mut ppu), expected(PALETTE, [3, 2, 1, 0]));
        }

        /// ...and the tile-data bank, which is a different bit of the same byte.
        #[test]
        fn bg_attributes_select_the_tile_data_bank() {
            let mut ppu = ppu_with_tile(ColorMode::Cgb);
            set_palette(&mut ppu, 0, 0, PALETTE);
            // A different tile 0 in bank 1: every pixel colour index 3.
            ppu.vram[VRAM_BANK_SIZE] = 0xFF;
            ppu.vram[VRAM_BANK_SIZE + 1] = 0xFF;
            ppu.vram[VRAM_BANK_SIZE + 0x1800] = 0x08; // attribute: tile data from bank 1

            assert_eq!(scanline(&mut ppu), vec![LcdColor::from_rgb555(PALETTE[3]); 8]);
        }

        /// DMG rendering must be untouched by any of this: no CGB palette, whatever is in RAM.
        #[test]
        fn dmg_still_renders_through_its_shades_alone() {
            let mut ppu = ppu_with_tile(ColorMode::Dmg);
            set_palette(&mut ppu, 0, 0, PALETTE); // must be ignored entirely
            ppu.palette.background_mut().set_from_byte(0b11_10_01_00); // identity

            let shades = [White, LightGray, DarkGray, Black].map(DMGColor::to_lcd);
            assert_eq!(scanline(&mut ppu), (0..8).map(|x| shades[x % 4]).collect::<Vec<_>>());
        }
    }

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