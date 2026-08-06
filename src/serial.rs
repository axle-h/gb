use bincode::{Decode, Encode};
use crate::cycles::MachineCycles;
use crate::activation::Activation;
use crate::schedule::DISABLED;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Serial {
    data: u8,
    transfer_enable: bool,
    master: bool,
    /// Absolute m-cycle at which the byte in flight began shifting out, or `None` when idle.
    started: Option<u64>,
    buffer: Option<Vec<u8>>,
    interrupt_pending: bool,
}

/// The serialised shape of a [`Serial`]: field-for-field what the `timer` save-state section has
/// always held. See [`crate::timer::TimerSnapshot`] for why C1 needed one.
#[derive(Debug, Clone, Decode, Encode)]
pub struct SerialSnapshot {
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
            started: None,
            buffer: None,
            interrupt_pending: false,
        }
    }
}

impl Serial {
    pub fn snapshot(&self, now: u64) -> SerialSnapshot {
        SerialSnapshot {
            data: self.data,
            transfer_enable: self.transfer_enable,
            master: self.master,
            state: match self.started {
                None => SerialState::Idle,
                Some(started) => {
                    SerialState::Transferring { cycles: MachineCycles::from_m(now - started) }
                }
            },
            buffer: self.buffer.clone(),
            interrupt_pending: self.interrupt_pending,
        }
    }

    pub fn restore(&mut self, snapshot: SerialSnapshot, now: u64) {
        self.data = snapshot.data;
        self.transfer_enable = snapshot.transfer_enable;
        self.master = snapshot.master;
        self.started = match snapshot.state {
            SerialState::Idle => None,
            // Saturating: a pre-C1 state restores against a clock that restarts at zero, so a
            // transfer already part-way through has no room behind `now` to have started in.
            SerialState::Transferring { cycles } => Some(now.saturating_sub(cycles.m_cycles())),
        };
        self.buffer = snapshot.buffer;
        self.interrupt_pending = snapshot.interrupt_pending;
    }

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

    /// **D9.** `SB` as the guest sees it *during* a transfer.
    ///
    /// The byte shifts out a bit at a time, and with no link cable attached a `1` shifts in behind
    /// each one — so a guest that reads `SB` mid-transfer sees the top bits already replaced. `gb`
    /// used to hold the written value flat and then jump to `0xFF` at completion, which is
    /// observable to anything that polls.
    ///
    /// ⚠️ **This is a read-side view and must stay one.** [`Serial::complete_transfer`] still
    /// buffers `self.data`, the byte the guest actually wrote — that is how blargg's output is
    /// captured (`serial_console_test`), and shifting the stored copy would corrupt it.
    pub fn data_at(&self, now: u64, fast: bool) -> u8 {
        let Some(started) = self.started else { return self.data };
        let bits = ((now.saturating_sub(started)) * 8 / Self::period(fast)).min(8) as u32;
        // `(data + 1) << n - 1` fills the vacated low bits with ones in one step. Widened to u32
        // because `0xFF` at eight bits shifted would overflow a u16.
        (((self.data as u32 + 1) << bits).wrapping_sub(1)) as u8
    }

    pub fn control(&self) -> u8 {
        let mut control = 0;
        if self.transfer_enable { control |= 0x80; }
        if self.master { control |= 0x01; }
        control
    }

    pub fn set_control(&mut self, control: u8, now: u64) {
        self.transfer_enable = (control & 0x80) != 0;
        self.master = (control & 0x01) != 0;

        if self.master && self.transfer_enable {
            self.started = Some(now);
        }
    }

    /// `fast` is the CGB's `SC` bit 1: a 32x faster shift clock. It lives in the MMU rather than
    /// here so the shipped `timer` save-state section keeps its shape — see `MMU::serial_fast`.
    fn period(fast: bool) -> u64 {
        if fast {
            MachineCycles::PER_SERIAL_BYTE_TRANSFER.m_cycles() / 32
        } else {
            MachineCycles::PER_SERIAL_BYTE_TRANSFER.m_cycles()
        }
    }

    /// Advance the shift clock to absolute m-cycle `now`.
    ///
    /// The transfer is timed from when it *started* rather than by accumulating a remainder, so
    /// flipping `SC` bit 1 part-way through still retimes the byte in flight — which is what the
    /// pre-C1 code did, since it recomputed the period on every call.
    #[inline]
    pub fn catch_up(&mut self, now: u64, fast: bool) {
        let Some(started) = self.started else { return };
        if now - started < Self::period(fast) {
            return;
        }
        self.complete_transfer();
    }

    /// Out of line: `MMU::update` runs once per CPU instruction and a link cable completes a byte
    /// roughly once every 512 of them, so this body is pure instruction-cache pressure on the path
    /// that matters. Same reasoning as [`crate::ppu::PPU::draw_pixels_to`] — see `CLAUDE.md`.
    #[cold]
    #[inline(never)]
    fn complete_transfer(&mut self) {
        if let Some(buffer) = self.buffer.as_mut() {
            buffer.push(self.data);
        }
        self.transfer_enable = false;
        self.data = 0xFF;
        self.interrupt_pending = true;
        self.started = None;
    }

    /// Absolute m-cycle at which the current transfer completes, or [`DISABLED`] when idle.
    pub fn next_event(&self, fast: bool) -> u64 {
        match self.started {
            None => DISABLED,
            Some(started) => started + Self::period(fast),
        }
    }
}

/// The serialised form of a transfer in flight: how far into it the shift clock has got. The live
/// [`Serial`] stores the stamp it started at instead, which is the same thing against a known
/// clock and one subtraction cheaper per instruction.
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