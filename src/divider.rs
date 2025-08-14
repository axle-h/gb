use crate::cycles::MachineCycles;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Divider {
    enabled: bool,
    value: u8,
    cycles_since_tick: MachineCycles,
}

impl Default for Divider {
    fn default() -> Self {
        Self {
            enabled: true,
            value: 0,
            cycles_since_tick: MachineCycles::ZERO,
        }
    }
}

impl Divider {
    pub fn enable(&mut self) {
        self.enabled = true;
    }

    pub fn disable(&mut self) {
        self.value = 0;
        self.enabled = false;
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn reset(&mut self) {
        self.value = 0;
    }

    pub fn value(&self) -> u8 {
        self.value
    }

    pub fn update(&mut self, cycles: MachineCycles) {
        if !self.enabled {
            return;
        }
        self.cycles_since_tick += cycles;
        while self.cycles_since_tick >= MachineCycles::PER_DIVIDER_TICK {
            self.cycles_since_tick -= MachineCycles::PER_DIVIDER_TICK;
            self.value = self.value.wrapping_add(1);
        }
    }
}