use bincode::{Decode, Encode};

/// The CGB's VRAM DMA controller — `HDMA1`-`HDMA5` (`FF51`-`FF55`).
///
/// Two transfer modes share one register set:
///
/// * **GDMA** (general purpose) copies the whole block at once, with the CPU stopped.
/// * **HDMA** copies `0x10` bytes at the start of each HBlank until the block is done.
///
/// # Accuracy caveat
///
/// `gb` renders mode 3 as a fixed 172 ticks, so it has no accurate mode-0 boundary to hang HDMA
/// off. A block is transferred **at the mode-3 to mode-0 transition** — correct in ordering and in
/// how many blocks land per frame, approximate in cycle placement. GDMA's CPU stall (8 M-cycles
/// per block in single speed) is likewise not modelled: the copy is instantaneous and the guest
/// loses no time. Both need the M-cycle work the plan defers in §0.2. Interleaving with OAM DMA is
/// out of scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Decode, Encode)]
pub struct Hdma {
    source: u16,
    destination: u16,
    /// Blocks of `0x10` bytes still to copy, minus one — the same encoding as `FF55` bits 0-6.
    remaining: u8,
    /// True while an HBlank-paced transfer is in flight.
    active: bool,
}

/// One `0x10`-byte block to copy: absolute source address and VRAM offset.
pub struct HdmaBlock {
    pub source: u16,
    pub destination: u16,
}

impl HdmaBlock {
    pub fn bytes(&self) -> impl Iterator<Item = (u16, u16)> + '_ {
        (0..0x10u16).map(move |i| (self.source.wrapping_add(i), self.destination.wrapping_add(i)))
    }
}

/// What a write to `FF55` asked for.
pub enum HdmaRequest {
    /// Copy this many `0x10`-byte blocks immediately.
    General(u8),
    /// An HBlank-paced transfer was started, or a running one cancelled. Nothing to do now.
    None,
}

impl Hdma {
    pub fn set_source_high(&mut self, value: u8) {
        self.source = (self.source & 0x00FF) | ((value as u16) << 8);
    }

    pub fn set_source_low(&mut self, value: u8) {
        // The low nibble is ignored: transfers are 16-byte aligned.
        self.source = (self.source & 0xFF00) | (value & 0xF0) as u16;
    }

    pub fn set_destination_high(&mut self, value: u8) {
        // Only bits 0-4 of the high byte matter; the destination is always inside VRAM.
        self.destination = (self.destination & 0x00FF) | (((value & 0x1F) as u16) << 8);
    }

    pub fn set_destination_low(&mut self, value: u8) {
        self.destination = (self.destination & 0xFF00) | (value & 0xF0) as u16;
    }

    /// `FF55` read-back: bit 7 clear while an HBlank transfer is running, set otherwise. Bits 0-6
    /// are the blocks left to copy, minus one — so `0xFF` means "idle", which is what a guest
    /// polls for.
    pub fn status(&self) -> u8 {
        if self.active {
            self.remaining & 0x7F
        } else {
            0x80 | (self.remaining & 0x7F)
        }
    }

    /// A write to `FF55`.
    pub fn request(&mut self, value: u8) -> HdmaRequest {
        let blocks = value & 0x7F;
        if value & 0x80 == 0 {
            if self.active {
                // Writing bit 7 clear during an HBlank transfer cancels it. The length left is
                // preserved and bit 7 now reads 1, which is how a guest detects the cancellation.
                self.active = false;
                return HdmaRequest::None;
            }
            self.remaining = 0xFF; // idle
            HdmaRequest::General(blocks + 1)
        } else {
            self.remaining = blocks;
            self.active = true;
            HdmaRequest::None
        }
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Called at the start of each HBlank. Yields the block to copy, if a transfer is running,
    /// and advances the pointers.
    pub fn next_block(&mut self) -> Option<HdmaBlock> {
        if !self.active {
            return None;
        }
        let block = HdmaBlock { source: self.source, destination: self.destination };
        self.source = self.source.wrapping_add(0x10);
        self.destination = (self.destination.wrapping_add(0x10)) & 0x1FFF;

        if self.remaining == 0 {
            self.active = false;
            self.remaining = 0xFF; // done: FF55 reads 0xFF
        } else {
            self.remaining -= 1;
        }
        Some(block)
    }

    /// Advance the pointers over a general-purpose transfer that has already been performed.
    pub fn advance(&mut self, blocks: u8) {
        self.source = self.source.wrapping_add((blocks as u16) << 4);
        self.destination = (self.destination.wrapping_add((blocks as u16) << 4)) & 0x1FFF;
    }

    /// Blocks a general-purpose transfer should copy, from the pointer it starts at.
    pub fn general_blocks(&self, blocks: u8) -> impl Iterator<Item = HdmaBlock> + '_ {
        (0..blocks as u16).map(move |i| HdmaBlock {
            source: self.source.wrapping_add(i << 4),
            destination: (self.destination.wrapping_add(i << 4)) & 0x1FFF,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_status_is_all_ones() {
        let hdma = Hdma::default();
        assert_eq!(hdma.status() & 0x80, 0x80, "bit 7 set means no transfer running");
    }

    #[test]
    fn addresses_are_sixteen_byte_aligned() {
        let mut hdma = Hdma::default();
        hdma.set_source_high(0x71);
        hdma.set_source_low(0x2F);
        hdma.set_destination_high(0xFF); // only bits 0-4 survive
        hdma.set_destination_low(0x3F);
        assert_eq!(hdma.source, 0x7120);
        assert_eq!(hdma.destination, 0x1F30);
    }

    #[test]
    fn hblank_transfer_runs_one_block_at_a_time_then_reports_done() {
        let mut hdma = Hdma::default();
        hdma.set_source_high(0x40);
        matches!(hdma.request(0x80 | 2), HdmaRequest::None); // 3 blocks

        for expected in 0..3u16 {
            assert!(hdma.is_active());
            let block = hdma.next_block().expect("a block per HBlank");
            assert_eq!(block.source, 0x4000 + expected * 0x10);
            assert_eq!(block.destination, expected * 0x10);
        }
        assert!(!hdma.is_active(), "the transfer should be finished");
        assert_eq!(hdma.status(), 0xFF, "done reads as 0xFF");
        assert!(hdma.next_block().is_none());
    }

    #[test]
    fn clearing_bit_7_cancels_a_running_transfer_and_keeps_the_remainder() {
        let mut hdma = Hdma::default();
        hdma.request(0x80 | 5); // 6 blocks
        hdma.next_block();
        assert_eq!(hdma.status() & 0x80, 0, "bit 7 clear while running");

        hdma.request(0x00);
        assert!(!hdma.is_active());
        assert_eq!(hdma.status(), 0x80 | 4, "bit 7 set, four blocks still outstanding");
    }

    #[test]
    fn a_general_transfer_yields_every_block_at_once() {
        let mut hdma = Hdma::default();
        hdma.set_source_high(0x20);
        hdma.set_destination_high(0x10);
        let HdmaRequest::General(blocks) = hdma.request(0x03) else {
            panic!("bit 7 clear must request a general transfer");
        };
        assert_eq!(blocks, 4);

        let addresses: Vec<(u16, u16)> = hdma.general_blocks(blocks)
            .map(|b| (b.source, b.destination))
            .collect();
        assert_eq!(addresses, vec![(0x2000, 0x1000), (0x2010, 0x1010), (0x2020, 0x1020), (0x2030, 0x1030)]);
        assert_eq!(hdma.status(), 0xFF, "a general transfer completes immediately");
    }
}
