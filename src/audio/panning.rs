use crate::audio::sample::AudioSample;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ChannelPanning {
    pub left: bool,
    pub right: bool,
}

impl ChannelPanning {
    pub fn pan(&self, raw: f32) -> AudioSample {
        let left = if self.left { raw } else { 0.0 };
        let right = if self.right { raw } else { 0.0 };
        AudioSample::new(left, right)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Panning {
    pub channel1: ChannelPanning,
    pub channel2: ChannelPanning,
    pub channel3: ChannelPanning,
    pub channel4: ChannelPanning,
}

impl Panning {
    pub fn get_byte(&self) -> u8 {
        let mut byte = 0;
        if self.channel1.right { byte |= 0x01; }
        if self.channel2.right { byte |= 0x02; }
        if self.channel3.right { byte |= 0x04; }
        if self.channel4.right { byte |= 0x08; }
        if self.channel1.left { byte |= 0x10; }
        if self.channel2.left { byte |= 0x20; }
        if self.channel3.left { byte |= 0x40; }
        if self.channel4.left { byte |= 0x80; }
        byte
    }

    pub fn set_byte(&mut self, value: u8) {
        self.channel1.left = (value & 0x10) != 0;
        self.channel1.right = (value & 0x01) != 0;
        self.channel2.left = (value & 0x20) != 0;
        self.channel2.right = (value & 0x02) != 0;
        self.channel3.left = (value & 0x40) != 0;
        self.channel3.right = (value & 0x04) != 0;
        self.channel4.left = (value & 0x80) != 0;
        self.channel4.right = (value & 0x08) != 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_values() {
        let panning = Panning::default();
        assert_eq!(panning.get_byte(), 0);
    }

    #[test]
    fn set_all_channels_right() {
        let mut panning = Panning::default();
        panning.set_byte(0x0F); // All right channels
        assert_eq!(panning.get_byte(), 0x0F);
        let expected = ChannelPanning { left: false, right: true };
        assert_eq!(
            panning,
            Panning { channel1: expected, channel2: expected, channel3: expected, channel4: expected }
        );
    }

    #[test]
    fn set_all_channels_left() {
        let mut panning = Panning::default();
        panning.set_byte(0xF0); // All left channels
        assert_eq!(panning.get_byte(), 0xF0);
        let expected = ChannelPanning { left: true, right: false };
        assert_eq!(
            panning,
            Panning { channel1: expected, channel2: expected, channel3: expected, channel4: expected }
        );
    }

    #[test]
    fn set_all_channels_both_sides() {
        let mut panning = Panning::default();
        panning.set_byte(0xFF); // All channels both sides
        assert_eq!(panning.get_byte(), 0xFF);
        let expected = ChannelPanning { left: true, right: true };
        assert_eq!(
            panning,
            Panning { channel1: expected, channel2: expected, channel3: expected, channel4: expected }
        );
    }

    #[test]
    fn individual_channel_bits() {
        let mut register = Panning::default();

        // Test each bit individually
        register.set_byte(0x01); // Channel 1 right
        assert_eq!(register.get_byte(), 0x01);
        assert_eq!(register.channel1, ChannelPanning { left: false, right: true });

        register.set_byte(0x02); // Channel 2 right
        assert_eq!(register.get_byte(), 0x02);
        assert_eq!(register.channel2, ChannelPanning { left: false, right: true });

        register.set_byte(0x04); // Channel 3 right
        assert_eq!(register.get_byte(), 0x04);
        assert_eq!(register.channel3, ChannelPanning { left: false, right: true });

        register.set_byte(0x08); // Channel 4 right
        assert_eq!(register.get_byte(), 0x08);
        assert_eq!(register.channel4, ChannelPanning { left: false, right: true });

        register.set_byte(0x10); // Channel 1 left
        assert_eq!(register.get_byte(), 0x10);
        assert_eq!(register.channel1, ChannelPanning { left: true, right: false });

        register.set_byte(0x20); // Channel 2 left
        assert_eq!(register.get_byte(), 0x20);
        assert_eq!(register.channel2, ChannelPanning { left: true, right: false });

        register.set_byte(0x40); // Channel 3 left
        assert_eq!(register.get_byte(), 0x40);
        assert_eq!(register.channel3, ChannelPanning { left: true, right: false });

        register.set_byte(0x80); // Channel 4 left
        assert_eq!(register.get_byte(), 0x80);
        assert_eq!(register.channel4, ChannelPanning { left: true, right: false });
    }
}
