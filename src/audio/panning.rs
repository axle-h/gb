use crate::audio::channel::Channel;

/// FF25 — NR51: Sound panning
#[derive(Debug, Clone, Default)]
pub struct AudioPanningRegister {
    channel1_left: bool,
    channel1_right: bool,
    channel2_left: bool,
    channel2_right: bool,
    channel3_left: bool,
    channel3_right: bool,
    channel4_left: bool,
    channel4_right: bool,
}

impl AudioPanningRegister {
    pub fn get(&self) -> u8 {
        let mut byte = 0;
        if self.channel1_right { byte |= 0x01; }
        if self.channel2_right { byte |= 0x02; }
        if self.channel3_right { byte |= 0x04; }
        if self.channel4_right { byte |= 0x08; }
        if self.channel1_left { byte |= 0x10; }
        if self.channel2_left { byte |= 0x20; }
        if self.channel3_left { byte |= 0x40; }
        if self.channel4_left { byte |= 0x80; }
        byte
    }

    pub fn set(&mut self, value: u8) {
        self.channel1_left = (value & 0x10) != 0;
        self.channel1_right = (value & 0x01) != 0;
        self.channel2_left = (value & 0x20) != 0;
        self.channel2_right = (value & 0x02) != 0;
        self.channel3_left = (value & 0x40) != 0;
        self.channel3_right = (value & 0x04) != 0;
        self.channel4_left = (value & 0x80) != 0;
        self.channel4_right = (value & 0x08) != 0;
    }

    pub fn panning(&self, channel: Channel) -> ChannelPanning {
        use Channel::*;
        match channel {
            Channel1 => ChannelPanning {
                left: self.channel1_left,
                right: self.channel1_right,
            },
            Channel2 => ChannelPanning {
                left: self.channel2_left,
                right: self.channel2_right,
            },
            Channel3 => ChannelPanning {
                left: self.channel3_left,
                right: self.channel3_right,
            },
            Channel4 => ChannelPanning {
                left: self.channel4_left,
                right: self.channel4_right,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ChannelPanning {
    pub left: bool,
    pub right: bool,
}

impl ChannelPanning {
    pub fn pan(&self, raw: f32) -> (f32, f32) {
        let left = if self.left { raw } else { 0.0 };
        let right = if self.right { raw } else { 0.0 };
        (left, right)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_values() {
        let register = AudioPanningRegister::default();
        assert_eq!(register.get(), 0);
        for channel in Channel::all() {
            assert_eq!(register.panning(channel), ChannelPanning::default());
        }
    }

    #[test]
    fn set_all_channels_right() {
        let mut register = AudioPanningRegister::default();
        register.set(0x0F); // All right channels
        assert_eq!(register.get(), 0x0F);
        let expected = ChannelPanning { left: false, right: true };
        for channel in Channel::all() {
            assert_eq!(register.panning(channel), expected);
        }
    }

    #[test]
    fn set_all_channels_left() {
        let mut register = AudioPanningRegister::default();
        register.set(0xF0); // All left channels
        assert_eq!(register.get(), 0xF0);
        let expected = ChannelPanning { left: true, right: false };
        for channel in Channel::all() {
            assert_eq!(register.panning(channel), expected);
        }
    }

    #[test]
    fn set_all_channels_both_sides() {
        let mut register = AudioPanningRegister::default();
        register.set(0xFF); // All channels both sides
        assert_eq!(register.get(), 0xFF);
        let expected = ChannelPanning { left: true, right: true };
        for channel in Channel::all() {
            assert_eq!(register.panning(channel), expected);
        }
    }

    #[test]
    fn individual_channel_bits() {
        let mut register = AudioPanningRegister::default();

        // Test each bit individually
        register.set(0x01); // Channel 1 right
        assert_eq!(register.get(), 0x01);
        assert_eq!(register.panning(Channel::Channel1), ChannelPanning { left: false, right: true });

        register.set(0x02); // Channel 2 right
        assert_eq!(register.get(), 0x02);
        assert_eq!(register.panning(Channel::Channel2), ChannelPanning { left: false, right: true });

        register.set(0x04); // Channel 3 right
        assert_eq!(register.get(), 0x04);
        assert_eq!(register.panning(Channel::Channel3), ChannelPanning { left: false, right: true });

        register.set(0x08); // Channel 4 right
        assert_eq!(register.get(), 0x08);
        assert_eq!(register.panning(Channel::Channel4), ChannelPanning { left: false, right: true });

        register.set(0x10); // Channel 1 left
        assert_eq!(register.get(), 0x10);
        assert_eq!(register.panning(Channel::Channel1), ChannelPanning { left: true, right: false });

        register.set(0x20); // Channel 2 left
        assert_eq!(register.get(), 0x20);
        assert_eq!(register.panning(Channel::Channel2), ChannelPanning { left: true, right: false });

        register.set(0x40); // Channel 3 left
        assert_eq!(register.get(), 0x40);
        assert_eq!(register.panning(Channel::Channel3), ChannelPanning { left: true, right: false });

        register.set(0x80); // Channel 4 left
        assert_eq!(register.get(), 0x80);
        assert_eq!(register.panning(Channel::Channel4), ChannelPanning { left: true, right: false });
    }
}
