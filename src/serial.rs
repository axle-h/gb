use bincode::{Decode, Encode};
use crate::cycles::MachineCycles;
use crate::activation::Activation;

#[derive(Debug, Clone, Eq, PartialEq, Decode, Encode)]
pub struct Serial {
    data: u8,
    transfer_enable: bool,
    master: bool,
    state: SerialState,
    buffer: Option<Vec<u8>>,
    interrupt_pending: bool,
}

impl Default for Serial {
    fn default() -> Self {
        Self {
            data: 0xFF,
            transfer_enable: false,
            master: false,
            state: SerialState::Idle,
            buffer: None,
            interrupt_pending: false,
        }
    }
}

impl Serial {
    pub fn enable_buffer(&mut self) {
        self.buffer = Some(Vec::new());
    }

    pub fn buffered_bytes(&self) -> Option<&[u8]> {
        self.buffer.as_deref()
    }

    pub fn set_data(&mut self, data: u8) {
        self.data = data;
    }

    pub fn get_data(&self) -> u8 {
        self.data
    }

    pub fn control(&self) -> u8 {
        let mut control = 0;
        if self.transfer_enable { control |= 0x80; }
        if self.master { control |= 0x01; }
        control
    }

    pub fn set_control(&mut self, control: u8) {
        self.transfer_enable = (control & 0x80) != 0;
        self.master = (control & 0x01) != 0;

        if self.master && self.transfer_enable {
            self.state = SerialState::Transferring { cycles: MachineCycles::ZERO };
        }
    }

    pub fn update(&mut self, delta_cycles: MachineCycles) {
        if let SerialState::Transferring { cycles } = self.state {
            let cycles = cycles + delta_cycles;
            self.state = if cycles >= MachineCycles::PER_SERIAL_BYTE_TRANSFER {
                if let Some(buffer) = self.buffer.as_mut() {
                    buffer.push(self.data);
                }
                self.transfer_enable = false;
                self.data = 0xFF;
                self.interrupt_pending = true;
                SerialState::Idle
            } else {
                SerialState::Transferring { cycles }
            };
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Decode, Encode)]
enum SerialState {
    #[default]
    Idle,
    Transferring { cycles: MachineCycles },
}

impl Activation for Serial {
    fn is_activation_pending(&self) -> bool {
        self.interrupt_pending
    }

    fn clear_activation(&mut self) {
        self.interrupt_pending = false
    }
}