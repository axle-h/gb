use crate::audio::channel::Channel;

/// FF26 — NR52: Audio master control
#[derive(Debug, Clone, Default)]
pub struct MasterControlRegister {
    enable: bool,
    channel1_enable: bool,
    channel2_enable: bool,
    channel3_enable: bool,
    channel4_enable: bool,
}

impl MasterControlRegister {
    pub fn get(&self) -> u8 {
        let mut byte = 0;
        if self.enable {
            byte |= 0x80; // Bit 7: Master enable
        }
        if self.channel1_enable {
            byte |= 0x01; // Bit 0: Channel 1 enable
        }
        if self.channel2_enable {
            byte |= 0x02; // Bit 1: Channel 2 enable
        }
        if self.channel3_enable {
            byte |= 0x04; // Bit 2: Channel 3 enable
        }
        if self.channel4_enable {
            byte |= 0x08; // Bit 3: Channel 4 enable
        }
        byte
    }

    pub fn set(&mut self, value: u8) {
        self.enable = (value & 0x80) != 0; // Bit 7: Master enable
        // the rest of this register is not writable
    }

    pub fn is_enabled(&self) -> bool {
        self.enable
    }

    pub fn set_channel_enabled(&mut self, channel: Channel, enabled: bool) {
        use Channel::*;
        match channel {
            Channel1 => self.channel1_enable = enabled,
            Channel2 => self.channel2_enable = enabled,
            Channel3 => self.channel3_enable = enabled,
            Channel4 => self.channel4_enable = enabled,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_control_default() {
        let audio = MasterControlRegister::default();
        assert_eq!(audio.get(), 0x00);
        assert!(!audio.is_enabled());
    }

    #[test]
    fn audio_control_set_master_enable() {
        let mut audio = MasterControlRegister::default();

        // Test setting master enable bit (bit 7)
        audio.set(0x80);
        assert!(audio.is_enabled());
        assert_eq!(audio.get(), 0x80);

        // Test clearing master enable bit
        audio.set(0x00);
        assert!(!audio.is_enabled());
        assert_eq!(audio.get(), 0x00);
    }

    #[test]
    fn audio_control_set_ignores_lower_bits() {
        let mut audio = MasterControlRegister::default();

        // Set master enable and try to set lower bits (which should be ignored)
        audio.set(0xFF);
        assert!(audio.is_enabled());
        assert_eq!(audio.get(), 0x80); // Only bit 7 should be set

        // Try to set only lower bits without master enable
        audio.set(0x0F);
        assert!(!audio.is_enabled());
        assert_eq!(audio.get(), 0x00); // Should be cleared
    }

    #[test]
    fn set_channel_enabled_individual_channels() {
        let mut audio = MasterControlRegister::default();

        // Test Channel 1
        audio.set_channel_enabled(Channel::Channel1, true);
        assert_eq!(audio.get(), 0x01);
        audio.set_channel_enabled(Channel::Channel1, false);
        assert_eq!(audio.get(), 0x00);

        // Test Channel 2
        audio.set_channel_enabled(Channel::Channel2, true);
        assert_eq!(audio.get(), 0x02);
        audio.set_channel_enabled(Channel::Channel2, false);
        assert_eq!(audio.get(), 0x00);

        // Test Channel 3
        audio.set_channel_enabled(Channel::Channel3, true);
        assert_eq!(audio.get(), 0x04);
        audio.set_channel_enabled(Channel::Channel3, false);
        assert_eq!(audio.get(), 0x00);

        // Test Channel 4
        audio.set_channel_enabled(Channel::Channel4, true);
        assert_eq!(audio.get(), 0x08);
        audio.set_channel_enabled(Channel::Channel4, false);
        assert_eq!(audio.get(), 0x00);
    }

    #[test]
    fn set_multiple_channels_enabled() {
        let mut audio = MasterControlRegister::default();
        audio.set(0x80); // Enable master

        // Enable multiple channels
        audio.set_channel_enabled(Channel::Channel1, true);
        audio.set_channel_enabled(Channel::Channel3, true);

        let value = audio.get();
        assert_eq!(value, 0x85); // Master enable (0x80) + Channel1 (0x01) + Channel3 (0x04)

        // Enable all channels
        audio.set_channel_enabled(Channel::Channel2, true);
        audio.set_channel_enabled(Channel::Channel4, true);

        let value = audio.get();
        assert_eq!(value, 0x8F); // Master enable (0x80) + all channels (0x0F)
    }
}
