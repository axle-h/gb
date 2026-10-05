//! The Super Game Boy, as far as a cartridge can tell it is on one and a player can see it: the
//! command packets sent through `P1`, the joypad number `MLT_REQ` makes readable, and the palettes
//! and per-cell attributes the SNES paints the DMG picture with.
//!
//! Not modelled: the border (`CHR_TRN`, `PCT_TRN`), anything run on the SNES (`DATA_SND`, `JUMP`),
//! sound (`SOUND`, `SOU_TRN`), and the SGB1's clock, 2.4% faster than a DMG's.
//!
//! The SNES colours what the DMG finished, so the picture is taken from the PPU's DMG frame once a
//! frame, at VBlank, and its four greys read back as the shades `BGP`/`OBP` produced.

use bincode::{Decode, Encode};
use crate::lcd_palette::{DMGColor, LcdColor};
use crate::ppu::{LCD_HEIGHT, LCD_WIDTH};

const COLUMNS: usize = LCD_WIDTH / 8;
const ROWS: usize = LCD_HEIGHT / 8;
const PACKET_BITS: u16 = 16 * 8;
const SYSTEM_PALETTES: usize = 512;
const ATTRIBUTE_FILES: usize = 45;
const ATTRIBUTE_FILE_BYTES: usize = COLUMNS * ROWS / 4;
/// A VRAM transfer: the first 256 tiles of the screen, row by row.
const TRANSFER_BYTES: usize = 256 * 16;

/// The palette the SGB paints a cartridge with until it says otherwise, 1-A.
const DEFAULT_PALETTE: [u16; 4] = [0x67BF, 0x265B, 0x10B5, 0x2866];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Encode, Decode)]
enum Mask {
    #[default]
    Off,
    Freeze,
    Black,
    Colour0,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
enum Transfer {
    Palettes,
    AttributeFiles,
}

/// Everything a save state keeps.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
struct State {
    /// The cartridge's header asks for SGB functions, without which the SNES ignores every packet.
    enabled: bool,
    /// Bits of the packet coming in, from a reset pulse until its stop bit.
    receiving: Option<u16>,
    /// A pulse is taken only after both lines have gone high since the last.
    ready_for_pulse: bool,
    /// The `P1` select lines last written, bits 4-5.
    lines: u8,
    packet: [u8; 16],
    /// The packets of the command so far.
    command: Vec<u8>,
    players: u8,
    player: u8,
    palettes: [[u16; 4]; 4],
    system_palettes: Vec<[u16; 4]>,
    /// A palette number per 8x8 cell.
    attributes: [u8; COLUMNS * ROWS],
    attribute_files: Vec<[u8; ATTRIBUTE_FILE_BYTES]>,
    mask: Mask,
    /// A VRAM transfer waiting on the frame after its command, and the VBlanks until it is whole.
    transfer: Option<(Transfer, u8)>,
}

pub const SGB_SECTION_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sgb {
    state: State,
    /// The picture as the SNES shows it.
    screen: Box<[LcdColor; LCD_WIDTH * LCD_HEIGHT]>,
}

impl Sgb {
    pub fn new(cart: &[u8]) -> Self {
        let enabled = cart.get(0x146) == Some(&0x03) && cart.get(0x14B) == Some(&0x33);
        Self {
            state: State {
                enabled,
                receiving: None,
                ready_for_pulse: false,
                lines: 0x30,
                packet: [0; 16],
                command: Vec::new(),
                players: 1,
                player: 0,
                palettes: [DEFAULT_PALETTE; 4],
                system_palettes: vec![[0; 4]; SYSTEM_PALETTES],
                attributes: [0; COLUMNS * ROWS],
                attribute_files: vec![[0; ATTRIBUTE_FILE_BYTES]; ATTRIBUTE_FILES],
                mask: Mask::Off,
                transfer: None,
            },
            screen: Box::new([LcdColor::from_rgb555(DEFAULT_PALETTE[0]); LCD_WIDTH * LCD_HEIGHT]),
        }
    }

    pub fn screen(&self) -> &[LcdColor; LCD_WIDTH * LCD_HEIGHT] {
        &self.screen
    }

    /// `P1` as read: with both lines high and more than one player, the low nibble is the joypad
    /// number, `F` for the first; a pad other than the first has nothing held.
    pub fn read_joypad(&self, value: u8) -> u8 {
        if self.state.players == 1 || self.state.player == 0 {
            value
        } else {
            (value & 0x30) | if value & 0x30 == 0x30 { 0x0F - self.state.player } else { 0x0F }
        }
    }

    /// A write to `P1`, of which only the select lines reach the SNES.
    pub fn write_joypad(&mut self, value: u8) {
        let state = &mut self.state;
        let lines = value & 0x30;
        let previous = std::mem::replace(&mut state.lines, lines);
        match lines {
            0x00 => {
                state.receiving = Some(0);
                state.ready_for_pulse = false;
            }
            0x30 => {
                state.ready_for_pulse = true;
                if previous != 0x30 && state.players > 1 && state.receiving.is_none() {
                    state.player = (state.player + 1) & (state.players - 1);
                }
            }
            _ => {
                let Some(bits) = state.receiving.filter(|_| state.ready_for_pulse) else { return };
                state.ready_for_pulse = false;
                // P15 low is a 1, P14 low a 0.
                let one = lines == 0x10;
                if bits == PACKET_BITS {
                    state.receiving = None;
                    if !one {
                        self.packet_received();
                    }
                    return;
                }
                if one {
                    state.packet[usize::from(bits / 8)] |= 1 << (bits % 8);
                } else {
                    state.packet[usize::from(bits / 8)] &= !(1 << (bits % 8));
                }
                state.receiving = Some(bits + 1);
            }
        }
    }

    fn packet_received(&mut self) {
        if !self.state.enabled {
            return;
        }
        let packet = self.state.packet;
        if self.state.command.is_empty() && packet[0] & 0x07 == 0 {
            return;
        }
        self.state.command.extend_from_slice(&packet);
        if self.state.command.len() / 16 >= usize::from(self.state.command[0] & 0x07) {
            let command = std::mem::take(&mut self.state.command);
            self.run(&command);
        }
    }

    fn run(&mut self, data: &[u8]) {
        let state = &mut self.state;
        let colour = |at: usize| u16::from_le_bytes([data[at], data[at + 1]]);
        match data[0] >> 3 {
            // PAL01, PAL23, PAL03, PAL12: colour 0 for all, then three colours of each of two.
            command @ 0x00..=0x03 => {
                let (first, second) = [(0, 1), (2, 3), (0, 3), (1, 2)][usize::from(command)];
                for palette in &mut state.palettes {
                    palette[0] = colour(1);
                }
                for (palette, at) in [(first, 3), (second, 9)] {
                    for shade in 1..4 {
                        state.palettes[palette][shade] = colour(at + (shade - 1) * 2);
                    }
                }
            }
            0x04 => attribute_blocks(&mut state.attributes, data),
            0x05 => {
                for &line in data[2..].iter().take(usize::from(data[1])) {
                    let (at, palette) = (usize::from(line & 0x1F), (line >> 5) & 0x03);
                    for cell in 0..COLUMNS * ROWS {
                        let (x, y) = (cell % COLUMNS, cell / COLUMNS);
                        if if line & 0x80 != 0 { y == at } else { x == at } {
                            state.attributes[cell] = palette;
                        }
                    }
                }
            }
            0x06 => {
                let (after, before, on) = (data[1] & 0x03, (data[1] >> 2) & 0x03, (data[1] >> 4) & 0x03);
                let at = usize::from(data[2]);
                for cell in 0..COLUMNS * ROWS {
                    let coordinate = if data[1] & 0x40 != 0 { cell / COLUMNS } else { cell % COLUMNS };
                    state.attributes[cell] = match coordinate.cmp(&at) {
                        std::cmp::Ordering::Less => before,
                        std::cmp::Ordering::Equal => on,
                        std::cmp::Ordering::Greater => after,
                    };
                }
            }
            0x07 => {
                let (mut x, mut y) = (usize::from(data[1]), usize::from(data[2]));
                let count = usize::from(u16::from_le_bytes([data[3], data[4]])).min(COLUMNS * ROWS);
                let down = data[5] != 0;
                for i in 0..count {
                    let Some(&byte) = data.get(6 + i / 4) else { break };
                    if x < COLUMNS && y < ROWS {
                        state.attributes[y * COLUMNS + x] = (byte >> (6 - 2 * (i % 4))) & 0x03;
                    }
                    if down {
                        y += 1;
                        if y >= ROWS { y = 0; x += 1; }
                    } else {
                        x += 1;
                        if x >= COLUMNS { x = 0; y += 1; }
                    }
                }
            }
            0x0A => {
                for (palette, at) in state.palettes.iter_mut().zip([1, 3, 5, 7]) {
                    *palette = state.system_palettes[usize::from(colour(at) & 0x1FF)];
                }
                // The SNES shares the first palette's colour 0.
                let shared = state.palettes[0][0];
                for palette in &mut state.palettes {
                    palette[0] = shared;
                }
                self.attribute_set(data[9], data[9] & 0x80 != 0);
            }
            0x0B => state.transfer = Some((Transfer::Palettes, 2)),
            0x11 => {
                state.players = match data[1] & 0x03 { 1 => 2, 3 => 4, _ => 1 };
                state.player = 0;
            }
            0x15 => state.transfer = Some((Transfer::AttributeFiles, 2)),
            0x16 => self.attribute_set(data[1], true),
            0x17 => state.mask = [Mask::Off, Mask::Freeze, Mask::Black, Mask::Colour0][usize::from(data[1] & 0x03)],
            _ => {}
        }
    }

    /// `PAL_SET`'s and `ATTR_SET`'s last byte: bits 0-5 the attribute file, applied when `apply`,
    /// and bit 6 the mask to cancel.
    fn attribute_set(&mut self, value: u8, apply: bool) {
        let state = &mut self.state;
        if apply {
            let file = state.attribute_files[usize::from(value & 0x3F).min(ATTRIBUTE_FILES - 1)];
            for (cell, attribute) in state.attributes.iter_mut().enumerate() {
                *attribute = (file[cell / 4] >> (6 - 2 * (cell % 4))) & 0x03;
            }
        }
        if value & 0x40 != 0 {
            state.mask = Mask::Off;
        }
    }

    /// The DMG has finished a frame into `lcd`.
    #[cold]
    #[inline(never)]
    pub fn vblank(&mut self, lcd: &[LcdColor; LCD_WIDTH * LCD_HEIGHT]) {
        if let Some((transfer, frames)) = self.state.transfer {
            if frames > 1 {
                self.state.transfer = Some((transfer, frames - 1));
            } else {
                self.state.transfer = None;
                self.receive_vram(transfer, &capture(lcd));
            }
        }
        let state = &self.state;
        match state.mask {
            Mask::Freeze => {}
            Mask::Black => self.screen.fill(LcdColor::from_rgb555(0)),
            Mask::Colour0 => self.screen.fill(LcdColor::from_rgb555(state.palettes[0][0])),
            Mask::Off => {
                for (i, (out, &pixel)) in self.screen.iter_mut().zip(lcd.iter()).enumerate() {
                    let shade = shade(pixel);
                    let palette = if shade == 0 { 0 } else { state.attributes[i / LCD_WIDTH / 8 * COLUMNS + i % LCD_WIDTH / 8] };
                    *out = LcdColor::from_rgb555(state.palettes[usize::from(palette)][usize::from(shade)]);
                }
            }
        }
    }

    fn receive_vram(&mut self, transfer: Transfer, bytes: &[u8; TRANSFER_BYTES]) {
        match transfer {
            Transfer::Palettes => {
                for (palette, chunk) in self.state.system_palettes.iter_mut().zip(bytes.chunks_exact(8)) {
                    *palette = std::array::from_fn(|i| u16::from_le_bytes([chunk[i * 2], chunk[i * 2 + 1]]));
                }
            }
            Transfer::AttributeFiles => {
                for (file, chunk) in self.state.attribute_files.iter_mut().zip(bytes.chunks_exact(ATTRIBUTE_FILE_BYTES)) {
                    file.copy_from_slice(chunk);
                }
            }
        }
    }

    pub(crate) fn write_section(&self, writer: &mut crate::savestate::SectionWriter) -> Result<(), String> {
        writer.write(crate::savestate::labels::SGB, SGB_SECTION_VERSION, &self.state)
    }

    pub(crate) fn read_section(&mut self, reader: &crate::savestate::SectionReader) -> Result<(), String> {
        if let Some((_version, state)) = reader.read::<State>(crate::savestate::labels::SGB)? {
            self.state = state;
        }
        Ok(())
    }
}

/// `ATTR_BLK`: up to 18 rectangles, each colouring what is inside, on and outside its edge.
fn attribute_blocks(attributes: &mut [u8; COLUMNS * ROWS], data: &[u8]) {
    for block in data[2..].chunks_exact(6).take(usize::from(data[1])) {
        let control = block[0] & 0x07;
        let (inside, mut border, outside) = (block[1] & 0x03, (block[1] >> 2) & 0x03, (block[1] >> 4) & 0x03);
        // Asking for one side alone colours the edge with it too.
        let mut paint = control;
        match control {
            0b001 => { border = inside; paint |= 0b010; }
            0b100 => { border = outside; paint |= 0b010; }
            _ => {}
        }
        let (x1, y1, x2, y2) = (usize::from(block[2] & 0x1F), usize::from(block[3] & 0x1F), usize::from(block[4] & 0x1F), usize::from(block[5] & 0x1F));
        for (cell, attribute) in attributes.iter_mut().enumerate() {
            let (x, y) = (cell % COLUMNS, cell / COLUMNS);
            let within = (x1..=x2).contains(&x) && (y1..=y2).contains(&y);
            let on_edge = within && (x == x1 || x == x2 || y == y1 || y == y2);
            if on_edge && paint & 0b010 != 0 {
                *attribute = border;
            } else if within && !on_edge && paint & 0b001 != 0 {
                *attribute = inside;
            } else if !within && paint & 0b100 != 0 {
                *attribute = outside;
            }
        }
    }
}

/// The shade a DMG grey came from.
fn shade(pixel: LcdColor) -> u8 {
    [DMGColor::White, DMGColor::LightGray, DMGColor::DarkGray, DMGColor::Black]
        .into_iter()
        .position(|colour| colour.to_lcd() == pixel)
        .unwrap_or(0) as u8
}

/// What a VRAM transfer sends: the screen's first 256 tiles, twenty to a row, as 2bpp tile data.
fn capture(lcd: &[LcdColor; LCD_WIDTH * LCD_HEIGHT]) -> [u8; TRANSFER_BYTES] {
    let mut bytes = [0; TRANSFER_BYTES];
    for (tile, data) in bytes.chunks_exact_mut(16).enumerate() {
        let (column, row) = (tile % COLUMNS, tile / COLUMNS);
        for line in 0..8 {
            for x in 0..8 {
                let shade = shade(lcd[(row * 8 + line) * LCD_WIDTH + column * 8 + x]);
                data[line * 2] |= (shade & 1) << (7 - x);
                data[line * 2 + 1] |= (shade >> 1) << (7 - x);
            }
        }
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cycles::MachineCycles;
    use crate::game_boy::GameBoy;
    use crate::ram::ROM;

    /// A cartridge whose header asks for SGB functions.
    fn sgb() -> Sgb {
        let mut cart = vec![0; 0x150];
        cart[0x146] = 0x03;
        cart[0x14B] = 0x33;
        Sgb::new(&cart)
    }

    /// `packet` sent the way `SendSGBPacket` sends it.
    fn send(sgb: &mut Sgb, packet: [u8; 16]) {
        sgb.write_joypad(0x00);
        sgb.write_joypad(0x30);
        for byte in packet {
            for bit in 0..8 {
                sgb.write_joypad(if byte >> bit & 1 != 0 { 0x10 } else { 0x20 });
                sgb.write_joypad(0x30);
            }
        }
        sgb.write_joypad(0x20);
        sgb.write_joypad(0x30);
    }

    fn packet(bytes: &[u8]) -> [u8; 16] {
        let mut packet = [0; 16];
        packet[..bytes.len()].copy_from_slice(bytes);
        packet
    }

    /// One frame of `shade` everywhere, as the DMG finishes it.
    fn frame(shade: DMGColor) -> [LcdColor; LCD_WIDTH * LCD_HEIGHT] {
        [shade.to_lcd(); LCD_WIDTH * LCD_HEIGHT]
    }

    #[test]
    fn mlt_req_makes_the_second_pad_readable_on_the_next_rise() {
        let mut sgb = sgb();
        assert_eq!(sgb.read_joypad(0x3F) & 0x0F, 0x0F);
        send(&mut sgb, packet(&[0x11 << 3 | 1, 0x01]));
        assert_eq!(sgb.read_joypad(0x3F) & 0x0F, 0x0E, "the stop bit's rise moved on to pad 2");
        sgb.write_joypad(0x20);
        assert_eq!(sgb.read_joypad(0x2F) & 0x0F, 0x0F, "pad 2 holds nothing");
        sgb.write_joypad(0x30);
        assert_eq!(sgb.read_joypad(0x3F) & 0x0F, 0x0F, "and back to pad 1");
        send(&mut sgb, packet(&[0x11 << 3 | 1, 0x00]));
        assert_eq!(sgb.read_joypad(0x3F) & 0x0F, 0x0F);
    }

    #[test]
    fn a_cartridge_that_does_not_ask_for_sgb_functions_is_not_heard() {
        let mut sgb = Sgb::new(&[0; 0x150]);
        send(&mut sgb, packet(&[0x11 << 3 | 1, 0x01]));
        assert_eq!(sgb.read_joypad(0x3F) & 0x0F, 0x0F);
    }

    #[test]
    fn pal_set_paints_cells_with_transferred_palettes_by_attr_blk() {
        let mut sgb = sgb();
        send(&mut sgb, packet(&[0x0B << 3 | 1]));
        // System palette n is colours n+1..n+4, sent as tile data: shade 1 on every pixel makes
        // every byte pair 0xFF, 0x00.
        sgb.vblank(&frame(DMGColor::White));
        sgb.vblank(&frame(DMGColor::LightGray));
        assert_eq!(sgb.state.system_palettes[3], [0x00FF; 4]);

        sgb.state.system_palettes[1] = [0x7FFF, 0x001F, 0x03E0, 0x7C00];
        sgb.state.system_palettes[2] = [0x0000, 0x0011, 0x0022, 0x0033];
        send(&mut sgb, packet(&[0x0A << 3 | 1, 1, 0, 2, 0, 1, 0, 1, 0]));
        // The bottom-right cell to palette 1, inside and on the edge.
        send(&mut sgb, packet(&[0x04 << 3 | 1, 1, 0b011, 0b0101, 19, 17, 19, 17]));
        sgb.vblank(&frame(DMGColor::LightGray));
        assert_eq!(sgb.screen()[0], LcdColor::from_rgb555(0x001F), "palette 0");
        assert_eq!(sgb.screen()[LCD_WIDTH * LCD_HEIGHT - 1], LcdColor::from_rgb555(0x0011), "palette 1");
        sgb.vblank(&frame(DMGColor::White));
        assert_eq!(sgb.screen()[LCD_WIDTH * LCD_HEIGHT - 1], LcdColor::from_rgb555(0x7FFF), "colour 0 is palette 0's");
    }

    #[test]
    fn mask_en_freezes_blacks_and_lets_go() {
        let mut sgb = sgb();
        sgb.vblank(&frame(DMGColor::Black));
        let before = sgb.screen()[0];
        send(&mut sgb, packet(&[0x17 << 3 | 1, 1]));
        sgb.vblank(&frame(DMGColor::White));
        assert_eq!(sgb.screen()[0], before);
        send(&mut sgb, packet(&[0x17 << 3 | 1, 2]));
        sgb.vblank(&frame(DMGColor::White));
        assert_eq!(sgb.screen()[0], LcdColor::from_rgb555(0));
        send(&mut sgb, packet(&[0x17 << 3 | 1, 0]));
        sgb.vblank(&frame(DMGColor::White));
        assert_eq!(sgb.screen()[0], LcdColor::from_rgb555(DEFAULT_PALETTE[0]));
    }

    /// Pokémon Red finds the SGB, uploads its palettes and paints the title screen with them.
    #[test]
    fn pokemon_red_finds_the_sgb_and_paints_its_title_screen() {
        const W_ON_SGB: u16 = 0xCF1B;
        let mut gb = GameBoy::sgb(crate::test_fixtures::POKERED);
        gb.run(MachineCycles::PER_FRAME * 60 * 12);
        assert_eq!(gb.core().mmu().read(W_ON_SGB), 1);
        let sgb = gb.core().mmu().sgb().unwrap();
        assert!(sgb.state.system_palettes.iter().any(|palette| *palette != [0; 4]), "PAL_TRN arrived");
        let greys: Vec<_> = [DMGColor::White, DMGColor::LightGray, DMGColor::DarkGray, DMGColor::Black].map(DMGColor::to_lcd).into();
        let coloured = gb.core().mmu().display().iter().filter(|pixel| !greys.contains(pixel)).count();
        assert!(coloured > LCD_WIDTH * LCD_HEIGHT / 2, "{coloured} coloured pixels");

        let state = gb.save_state().unwrap();
        let mut restored = GameBoy::sgb(crate::test_fixtures::POKERED);
        restored.load_state(&state).unwrap();
        assert_eq!(restored.core().mmu().sgb().unwrap().state, sgb.state);
    }
}
