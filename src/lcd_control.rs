use crate::ppu::TILE_BYTES;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LcdControl {
    enabled: bool,
    window_tile_map: bool,
    window_enabled: bool,
    bg_tile_data: bool,
    bg_tile_map: bool,
    obj_size: bool,
    obj_enabled: bool,
    bg_window_enabled: bool,
}

impl LcdControl {
    pub fn set(&mut self, value: u8) {
        self.enabled = (value & 0x80) != 0;
        self.window_tile_map = (value & 0x40) != 0;
        self.window_enabled = (value & 0x20) != 0;
        self.bg_tile_data = (value & 0x10) != 0;
        self.bg_tile_map = (value & 0x08) != 0;
        self.obj_size = (value & 0x04) != 0;
        self.obj_enabled = (value & 0x02) != 0;
        self.bg_window_enabled = (value & 0x01) != 0;
    }

    pub fn get(&self) -> u8 {
        let mut value = 0;
        if self.enabled { value |= 0x80; }
        if self.window_tile_map { value |= 0x40; }
        if self.window_enabled { value |= 0x20; }
        if self.bg_tile_data { value |= 0x10; }
        if self.bg_tile_map { value |= 0x08; }
        if self.obj_size { value |= 0x04; }
        if self.obj_enabled { value |= 0x02; }
        if self.bg_window_enabled { value |= 0x01; }
        value
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn window_tile_map(&self) -> TileMapMode {
        TileMapMode::from_value(self.window_tile_map)
    }

    pub fn window_enabled(&self) -> bool {
        self.window_enabled && self.bg_window_enabled
    }

    pub fn tile_data_mode(&self) -> TileDataMode {
        TileDataMode::from_value(self.bg_tile_data)
    }

    pub fn background_tile_map(&self) -> TileMapMode {
        TileMapMode::from_value(self.bg_tile_map)
    }

    pub fn object_size(&self) -> ObjectSizeMode {
        ObjectSizeMode::from_value(self.obj_size)
    }

    pub fn objects_enabled(&self) -> bool {
        self.obj_enabled
    }

    pub fn background_enabled(&self) -> bool {
        self.bg_window_enabled
    }
}

impl Default for LcdControl {
    fn default() -> Self {
        Self {
            enabled: false,
            window_tile_map: false,
            window_enabled: false,
            bg_tile_data: false,
            bg_tile_map: false,
            obj_size: false,
            obj_enabled: false,
            bg_window_enabled: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TileDataMode { Lower, Upper }



impl TileDataMode {
    pub fn from_value(value: bool) -> Self {
        if value { Self::Lower } else { Self::Upper }
    }

    pub fn tile_address(&self, tile_index: u8) -> u16 {
        match self {
            Self::Lower => 0x8000 + tile_index as u16 * TILE_BYTES as u16,
            Self::Upper if tile_index < 0x80 => 0x9000 + tile_index as u16 * TILE_BYTES as u16,
            _ => 0x8800 + (tile_index as u16 - 0x80) * TILE_BYTES as u16, // signed tiles
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TileMapMode { Lower, Upper }

impl TileMapMode {
    pub fn from_value(value: bool) -> Self {
        if value { Self::Upper } else { Self::Lower }
    }

    pub fn base_address(&self) -> u16 {
        match self {
            Self::Lower => 0x9800,
            Self::Upper => 0x9C00,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectSizeMode { Single, Double }

impl ObjectSizeMode {
    pub fn from_value(value: bool) -> Self {
        if value { Self::Double } else { Self::Single }
    }

    pub fn height(&self) -> u8 {
        match self {
            Self::Single => 8,
            Self::Double => 16,
        }
    }

    pub fn width(&self) -> u8 {
        8 // Objects are always 8 pixels wide
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lcd_control_new() {
        let lcd = LcdControl::default();
        assert_eq!(lcd.get(), 0x00);
        assert!(!lcd.is_enabled());
        assert!(!lcd.window_enabled());
        assert!(!lcd.objects_enabled());
        assert!(!lcd.background_enabled());
    }

    #[test]
    fn test_lcd_control_set_individual_bits() {
        let mut lcd = LcdControl::default();

        // Test LCD enable bit (bit 7)
        lcd.set(0x80);
        assert!(lcd.is_enabled());
        assert_eq!(lcd.get(), 0x80);

        // Test window tile map bit (bit 6)
        lcd.set(0x40);
        assert_eq!(lcd.window_tile_map(), TileMapMode::Upper);
        assert_eq!(lcd.get(), 0x40);

        // Test window enable bit (bit 5)
        lcd.set(0x20);
        assert!(!lcd.window_enabled()); // Should be false because bg_window_enabled is false
        assert_eq!(lcd.get(), 0x20);

        // Test background tile data bit (bit 4)
        lcd.set(0x10);
        assert_eq!(lcd.tile_data_mode(), TileDataMode::Lower);
        assert_eq!(lcd.get(), 0x10);

        // Test background tile map bit (bit 3)
        lcd.set(0x08);
        assert_eq!(lcd.background_tile_map(), TileMapMode::Upper);
        assert_eq!(lcd.get(), 0x08);

        // Test object size bit (bit 2)
        lcd.set(0x04);
        assert_eq!(lcd.object_size(), ObjectSizeMode::Double);
        assert_eq!(lcd.get(), 0x04);

        // Test object enable bit (bit 1)
        lcd.set(0x02);
        assert!(lcd.objects_enabled());
        assert_eq!(lcd.get(), 0x02);

        // Test background/window enable bit (bit 0)
        lcd.set(0x01);
        assert!(lcd.background_enabled());
        assert_eq!(lcd.get(), 0x01);
    }

    #[test]
    fn test_lcd_control_window_enabled_logic() {
        let mut lcd = LcdControl::default();

        // Window should be disabled if only window_enabled is true
        lcd.set(0x20);
        assert!(!lcd.window_enabled());

        // Window should be disabled if only bg_window_enabled is true
        lcd.set(0x01);
        assert!(!lcd.window_enabled());

        // Window should be enabled only if both bits are set
        lcd.set(0x21);
        assert!(lcd.window_enabled());
    }

    #[test]
    fn test_lcd_control_all_bits_set() {
        let mut lcd = LcdControl::default();
        lcd.set(0xFF);

        assert!(lcd.is_enabled());
        assert_eq!(lcd.window_tile_map(), TileMapMode::Upper);
        assert!(lcd.window_enabled());
        assert_eq!(lcd.tile_data_mode(), TileDataMode::Lower);
        assert_eq!(lcd.background_tile_map(), TileMapMode::Upper);
        assert_eq!(lcd.object_size(), ObjectSizeMode::Double);
        assert!(lcd.objects_enabled());
        assert!(lcd.background_enabled());
        assert_eq!(lcd.get(), 0xFF);
    }

    #[test]
    fn test_tile_map_mode() {
        assert_eq!(TileMapMode::from_value(true), TileMapMode::Upper);
        assert_eq!(TileMapMode::from_value(false), TileMapMode::Lower);

        assert_eq!(TileMapMode::Lower.base_address(), 0x9800);
        assert_eq!(TileMapMode::Upper.base_address(), 0x9C00);
    }

    #[test]
    fn test_object_size_mode() {
        assert_eq!(ObjectSizeMode::from_value(true), ObjectSizeMode::Double);
        assert_eq!(ObjectSizeMode::from_value(false), ObjectSizeMode::Single);

        assert_eq!(ObjectSizeMode::Single.height(), 8);
        assert_eq!(ObjectSizeMode::Double.height(), 16);
        assert_eq!(ObjectSizeMode::Single.width(), 8);
        assert_eq!(ObjectSizeMode::Double.width(), 8);
    }

    #[test]
    fn test_lcd_control_round_trip() {
        // Test that setting and getting values preserves all bits correctly
        for value in 0..=255u8 {
            let mut lcd = LcdControl::default();
            lcd.set(value);
            assert_eq!(lcd.get(), value, "Failed round trip for value: 0x{:02X}", value);
        }
    }

    #[test]
    fn tile_data_mode_1() {
        let mode = TileDataMode::Lower;
        assert_eq!(mode.tile_address(0x00), 0x8000);
        assert_eq!(mode.tile_address(0x01), 0x8010);
        assert_eq!(mode.tile_address(0x80), 0x8800);
        assert_eq!(mode.tile_address(0xFF), 0x8FF0);
    }

    #[test]
    fn tile_data_mode_2() {
        let mode = TileDataMode::Upper;
        assert_eq!(mode.tile_address(0x00), 0x9000);
        assert_eq!(mode.tile_address(0x01), 0x9010);
        assert_eq!(mode.tile_address(0x80), 0x8800);
        assert_eq!(mode.tile_address(0xFF), 0x8FF0);
    }
}
