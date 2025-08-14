use crate::cycles::MachineCycles;
use crate::interrupt::InterruptSource;

#[derive(Debug, Clone, Default)]
pub struct Timer {
    enabled: bool,
    mode: TimerMode,
    value: u8,
    modulo: u8,
    cycles: MachineCycles,
    interrupt_pending: bool,
}

impl Timer {
    pub fn control(&self) -> u8 {
        self.mode as u8 | if self.enabled { 0b0100 } else { 0 }
    }

    pub fn set_control(&mut self, value: u8) {
        self.enabled = value & 0b0100 != 0;
        self.mode = TimerMode::from_repr(value & 0b11).unwrap_or_default();
    }

    pub fn value(&self) -> u8 {
        self.value
    }

    pub fn set_value(&mut self, value: u8) {
        self.value = value;
    }

    pub fn modulo(&self) -> u8 {
        self.modulo
    }

    pub fn set_modulo(&mut self, value: u8) {
        self.modulo = value;
    }

    pub fn update(&mut self, cycles: MachineCycles) {
        if !self.enabled {
            return;
        }

        self.cycles += cycles;

        let cycles_per_tick = self.mode.cycles_per_tick();
        while self.cycles >= cycles_per_tick {
            self.cycles -= cycles_per_tick;
            if self.value == 0xFF {
                self.value = self.modulo;
                self.interrupt_pending = true;
            } else {
                self.value = self.value.wrapping_add(1);
            }
        }
    }
}

impl InterruptSource for Timer {
    fn is_interrupt_pending(&self) -> bool {
        self.interrupt_pending
    }

    fn clear_interrupt(&mut self) {
        self.interrupt_pending = false;
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, strum_macros::FromRepr)]
#[repr(u8)]
enum TimerMode {
    #[default]
    M256 = 0,
    M4 = 1,
    M16 = 2,
    M64 = 3,
}

impl TimerMode {
    pub fn cycles_per_tick(self) -> MachineCycles {
        match self {
            TimerMode::M256 => MachineCycles(256),
            TimerMode::M4 => MachineCycles(4),
            TimerMode::M16 => MachineCycles(16),
            TimerMode::M64 => MachineCycles(64),
        }
    }
}

