use bincode::{Decode, Encode};
use crate::cycles::MachineCycles;

/// OAM DMA controller (`FF46`).
///
/// The transfer is **incremental**: one byte every 4 T-cycles, 160 bytes over 160 M-cycles, and
/// [`LcdDma::is_active`] stays true for the whole of it. The previous implementation cleared its
/// state *before* the copy ran, so the copy went through the ordinary mode-gated `write_oam` using
/// the PPU mode from the *previous* step — and silently discarded all 160 bytes whenever that mode
/// happened to be 2 or 3. Pokémon Red only ever DMAs during VBlank, which is the sole reason that
/// never showed up.
#[derive(Debug, Clone, PartialEq, Eq, Default, Decode, Encode)]
pub struct LcdDma {
    state: Option<LcdDmaState>,
    /// Last value written to `FF46`, so the register reads back.
    register: u8,
}

impl LcdDma {
    pub fn set(&mut self, value: u8) {
        self.register = value;
        self.state = Some(LcdDmaState {
            page: value,
            cycles: MachineCycles::ZERO,
            pos: 0,
        });
    }

    /// The byte last written to `FF46`.
    pub fn register(&self) -> u8 {
        self.register
    }

    /// Advance the transfer, returning the byte copies that fall due in this step. Each is a
    /// (source address, OAM offset) pair; the caller reads the source and writes OAM through the
    /// privileged path that bypasses the mode gate.
    pub fn update(&mut self, delta_machine_cycles: MachineCycles) -> Option<DmaTransfer> {
        let state = self.state.as_mut()?;

        state.cycles += delta_machine_cycles;
        // One byte per M-cycle, capped at the 160 the transfer copies.
        let target = (state.cycles.m_cycles() as usize).min(OAM_BYTES);
        let from = state.pos as usize;
        if target <= from {
            return None;
        }
        state.pos = target as u8;

        let transfer = DmaTransfer {
            source: state.source_base(),
            start: from,
            end: target,
        };

        if target >= OAM_BYTES {
            self.state = None;
        }
        Some(transfer)
    }

    pub fn is_active(&self) -> bool {
        self.state.is_some()
    }
}

/// A run of bytes that has fallen due this step.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct DmaTransfer {
    /// Address the transfer reads from, already mapped to a real region.
    pub source: u16,
    /// First OAM offset to write, inclusive.
    pub start: usize,
    /// Last OAM offset to write, exclusive.
    pub end: usize,
}

impl DmaTransfer {
    /// `(source address, OAM offset)` for each byte in this run.
    pub fn bytes(&self) -> impl Iterator<Item = (u16, u16)> + use<> {
        let source = self.source;
        (self.start..self.end).map(move |i| (source.wrapping_add(i as u16), i as u16))
    }
}

pub const OAM_BYTES: usize = 0xA0;

#[derive(Debug, Copy, Clone, PartialEq, Eq, Decode, Encode)]
pub struct LcdDmaState {
    /// The raw `FF46` value. Kept unmapped so the classification lives in one place.
    page: u8,
    cycles: MachineCycles,
    /// Bytes copied so far.
    pos: u8,
}

impl LcdDmaState {
    /// Map the source page to a real region, mirroring gambatte's `oamDmaInitSetup`
    /// (`memory.cpp:516-523`): `00-7F` ROM, `80-9F` VRAM, `A0-BF` SRAM, `C0-FF` WRAM with the
    /// echo-RAM wrap. The old code masked the page with `0xDF`, which cleared bit 5 and so turned
    /// `0x20`→`0x00`, `0x60`→`0x40` and — the damaging one — **`0xA0`→`0x80`, sending an
    /// SRAM-sourced transfer to VRAM**.
    fn source_base(&self) -> u16 {
        let page = if self.page >= 0xE0 {
            // Above 0xDF the bus wraps into echo RAM, which mirrors WRAM.
            self.page - 0x20
        } else {
            self.page
        };
        (page as u16) << 8
    }
}

/// The `dma` save-state section as written by version 1 — before A7 replaced the whole-transfer
/// model with an incremental one. Kept only to decode states written by that build; the section
/// version tells the two apart. See the rules at the top of `src/savestate/mod.rs`.
#[derive(Debug, Clone, Decode, Encode)]
pub struct LcdDmaV1 {
    state: Option<LcdDmaStateV1>,
}

#[derive(Debug, Copy, Clone, Decode, Encode)]
pub struct LcdDmaStateV1 {
    address: u16,
    cycles: MachineCycles,
}

impl From<LcdDmaV1> for LcdDma {
    fn from(old: LcdDmaV1) -> Self {
        match old.state {
            // v1 copied all 160 bytes in one go at the end of the transfer, so nothing had been
            // copied yet at any point where it recorded `Some`. `pos` therefore starts at 0, and
            // v1 stored the source as a full address rather than a page.
            Some(state) => Self {
                register: (state.address >> 8) as u8,
                state: Some(LcdDmaState {
                    page: (state.address >> 8) as u8,
                    cycles: state.cycles,
                    pos: 0,
                }),
            },
            None => Self::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_to_completion(dma: &mut LcdDma) -> Vec<(u16, u16)> {
        let mut copied = Vec::new();
        for _ in 0..OAM_BYTES {
            if let Some(transfer) = dma.update(MachineCycles::ONE) {
                copied.extend(transfer.bytes());
            }
        }
        copied
    }

    #[test]
    fn copies_160_bytes_one_per_machine_cycle() {
        let mut dma = LcdDma::default();
        dma.set(0xC1);
        assert!(dma.is_active());

        let copied = run_to_completion(&mut dma);

        assert_eq!(copied.len(), OAM_BYTES);
        assert_eq!(copied[0], (0xC100, 0x00));
        assert_eq!(copied[OAM_BYTES - 1], (0xC100 + 0x9F, 0x9F));
        assert!(!dma.is_active(), "the transfer should have finished");
    }

    /// `is_active` must stay true for the whole transfer, not just until the first update.
    #[test]
    fn stays_active_for_the_whole_transfer() {
        let mut dma = LcdDma::default();
        dma.set(0xC0);
        for i in 0..OAM_BYTES {
            assert!(dma.is_active(), "inactive after {i} bytes");
            dma.update(MachineCycles::ONE);
        }
        assert!(!dma.is_active());
    }

    /// A single oversized step still copies exactly 160 bytes and then finishes.
    #[test]
    fn a_large_step_completes_the_transfer_exactly_once() {
        let mut dma = LcdDma::default();
        dma.set(0xC0);
        let transfer = dma.update(MachineCycles::from_m(10_000)).unwrap();
        assert_eq!(transfer.bytes().count(), OAM_BYTES);
        assert!(!dma.is_active());
        assert!(dma.update(MachineCycles::ONE).is_none());
    }

    /// The old `& 0xDF` mask corrupted three source ranges. `0xA0` is the damaging one: an
    /// SRAM-sourced transfer became a VRAM-sourced one.
    #[test]
    fn source_pages_are_not_masked() {
        for (page, expected) in [
            (0x00u8, 0x0000u16), // ROM
            (0x20, 0x2000),      // ROM — was 0x0000
            (0x60, 0x6000),      // ROM — was 0x4000
            (0x80, 0x8000),      // VRAM
            (0xA0, 0xA000),      // SRAM — was 0x8000
            (0xC0, 0xC000),      // WRAM
            (0xDF, 0xDF00),      // WRAM, top
        ] {
            let mut dma = LcdDma::default();
            dma.set(page);
            let transfer = dma.update(MachineCycles::ONE).unwrap();
            assert_eq!(transfer.source, expected, "page {page:02X}");
        }
    }

    /// Pages at or above 0xE0 address echo RAM, which mirrors WRAM 0x2000 lower.
    #[test]
    fn echo_ram_pages_wrap_into_work_ram() {
        let mut dma = LcdDma::default();
        dma.set(0xE0);
        assert_eq!(dma.update(MachineCycles::ONE).unwrap().source, 0xC000);

        let mut dma = LcdDma::default();
        dma.set(0xFF);
        assert_eq!(dma.update(MachineCycles::ONE).unwrap().source, 0xDF00);
    }

    #[test]
    fn register_reads_back() {
        let mut dma = LcdDma::default();
        assert_eq!(dma.register(), 0);
        dma.set(0xC3);
        assert_eq!(dma.register(), 0xC3);
    }
}
