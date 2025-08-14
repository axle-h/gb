use crate::cycles::MachineCycles;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LcdDma {
    state: Option<LcdDmaState>,
}

impl LcdDma {
    pub fn set(&mut self, value: u8) {
        self.state = Some(LcdDmaState { address: ((value & 0xDF) as u16) << 8, cycles: MachineCycles::ZERO });
    }

    pub fn update(&mut self, delta_machine_cycles: MachineCycles) -> Option<DmaTransfer> {
        if let Some(state) = &mut self.state {
            state.cycles += delta_machine_cycles;
            if state.cycles >= DMA_TRANSFER_CYCLES {
                // Transfer complete, reset state
                let transfer = DmaTransfer { address: state.address };
                self.state = None;
                Some(transfer)
            } else {
                // still in transfer
                // TODO implement partial transfer logic
                None
            }
        } else {
            // no transfer in progress
            None
        }
    }

    pub fn is_active(&self) -> bool {
        self.state.is_some()
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct DmaTransfer {
    pub address: u16
}

const DMA_TRANSFER_CYCLES: MachineCycles = MachineCycles::of_machine(160);

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct LcdDmaState {
    address: u16,
    cycles: MachineCycles
}