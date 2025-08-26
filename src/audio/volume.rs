/// NRX2 - Volume and Envelope Register
/// This register controls the digital amplitude of the “high” part of the pulse, and the sweep applied to that setting.
#[derive(Debug, Clone, Default)]
pub struct VolumeAndEnvelopeRegister {
    /// bits 4-7: Initial volume:
    /// How loud the channel initially is.
    /// Note that these bits are readable, but are not updated by the envelope functionality!
    initial_volume: u8,
    /// bit 3: Envelope direction:
    /// The envelope’s direction; 0 = decrease volume over time, 1 = increase volume over time.
    envelope_direction: bool,
    /// bits 0-2: Envelope sweep pace:
    /// The envelope ticks at 64 Hz, and the channel’s envelope will be increased / decreased (depending on bit 3) every Sweep pace of those ticks.
    /// A setting of 0 disables the envelope.
    sweep_pace: u8,
}

impl VolumeAndEnvelopeRegister {
    pub fn get(&self) -> u8 {
        let mut byte = 0;
        byte |= (self.initial_volume & 0x0F) << 4; // bits 4-7: Initial volume
        if self.envelope_direction { byte |= 0x08; } // bit 3: Envelope direction
        byte |= self.sweep_pace & 0x07; // bits 0-2: Sweep pace
        byte
    }

    pub fn set(&mut self, value: u8) {
        self.initial_volume = (value >> 4) & 0x0F; // bits 4-7
        self.envelope_direction = (value & 0x08) != 0; // bit 3
        self.sweep_pace = value & 0x07; // bits 0-2
    }

    pub fn initial_volume(&self) -> u8 {
        self.initial_volume
    }

    pub fn envelope_direction(&self) -> bool {
        self.envelope_direction
    }

    pub fn sweep_pace(&self) -> u8 {
        if self.sweep_pace == 0 { 8 } else { self.sweep_pace }
    }
}

#[derive(Debug, Clone)]
pub struct EnvelopeFunction {
    register: VolumeAndEnvelopeRegister,
    current_volume: u8,
    period_counter: u8,
}

impl Default for EnvelopeFunction {
    fn default() -> Self {
        let mut default = Self {
            current_volume: 0,
            period_counter: 0,
            register: VolumeAndEnvelopeRegister::default()
        };
        default.reset();
        default
    }
}

impl EnvelopeFunction {
    pub const MAX_VOLUME: u8 = 0xF;

    pub fn dac_enabled(&self) -> bool {
        self.register.initial_volume != 0 || self.register.envelope_direction
    }

    pub fn reset(&mut self) {
        self.current_volume = self.register.initial_volume;
        self.period_counter = self.register.sweep_pace();
    }

    pub fn step(&mut self) {
        if self.register.sweep_pace == 0 {
            return; // Envelope is disabled
        }

        if self.period_counter > 0 {
            self.period_counter -= 1;
        }

        if self.period_counter != 0 {
            return;
        }

        self.period_counter = self.register.sweep_pace();

        if self.register.envelope_direction {
            // Increase volume
            if self.current_volume < Self::MAX_VOLUME {
                self.current_volume += 1;
            }
        } else {
            // Decrease volume
            if self.current_volume > 0 {
                self.current_volume -= 1;
            }
        }
    }

    pub fn current_volume(&self) -> u8 {
        self.current_volume
    }

    pub fn register(&self) -> &VolumeAndEnvelopeRegister {
        &self.register
    }

    pub fn register_mut(&mut self) -> &mut VolumeAndEnvelopeRegister {
        &mut self.register
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_values() {
        let register = VolumeAndEnvelopeRegister::default();
        assert_eq!(register.get(), 0);
        assert_eq!(register.initial_volume(), 0);
        assert!(!register.envelope_direction());
        assert_eq!(register.sweep_pace(), 8);
    }

    #[test]
    fn set_and_get_initial_volume() {
        let mut register = VolumeAndEnvelopeRegister::default();
        register.set(0b11110000); // Set initial volume to 15
        assert_eq!(register.get(), 0b11110000);
        assert_eq!(register.initial_volume(), 15);
    }

    #[test]
    fn set_and_get_envelope_direction() {
        let mut register = VolumeAndEnvelopeRegister::default();
        register.set(0b00001000); // Set envelope direction to increase
        assert_eq!(register.get(), 0b00001000);
        assert!(register.envelope_direction());
    }

    #[test]
    fn set_and_get_sweep_pace() {
        let mut register = VolumeAndEnvelopeRegister::default();
        register.set(0b00000011); // Set sweep pace to 3
        assert_eq!(register.get(), 0b00000011);
        assert_eq!(register.sweep_pace(), 3);
    }

    #[test]
    fn set_and_get_all() {
        let mut register = VolumeAndEnvelopeRegister::default();
        register.set(0b11111111); // Set all bits
        assert_eq!(register.get(), 0b11111111);
        assert_eq!(register.initial_volume(), 15);
        assert!(register.envelope_direction());
        assert_eq!(register.sweep_pace(), 7);
    }
}