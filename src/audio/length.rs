#[derive(Debug, Clone)]
pub struct LengthTimer {
    offset: u16,
    value: u16
}

impl LengthTimer {
    pub fn square_or_noise_channel() -> Self {
        Self::new(64)
    }

    pub fn wave_channel() -> Self {
        Self::new(256)
    }

    fn new(offset: u16) -> Self {
        Self { value: offset, offset }
    }

    pub fn reset(&mut self, initial_length_timer: u8) {
        self.value = self.offset - initial_length_timer as u16;
    }

    pub fn restart_from_max_if_expired(&mut self) {
        if self.value == 0 {
            self.reset(0x00);
        }
    }

    pub fn step(&mut self) -> bool {
        if self.value > 0 {
            self.value -= 1;
        }
        self.value == 0
    }
}