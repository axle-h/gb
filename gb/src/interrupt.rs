use bincode::{Decode, Encode};
use strum::IntoEnumIterator;

/// The five interrupt bits, in hardware's own layout: bit 0 VBlank … bit 4 Joypad.
/// <https://gbdev.io/pandocs/Interrupts.html#ffff--ie-interrupt-enable>
///
/// **A bitmask, not five `bool`s (C7).** `MMU::update` polls this once per CPU instruction and
/// `Core::interrupt` reads it again; as separate fields, "is anything pending?" was five field
/// reads and five branches each time. As a mask it is one `and`, and picking the winner is
/// `trailing_zeros` — with the priority order falling out of the bit order for free, because
/// hardware numbers them highest-priority-first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct InterruptFlags(u8);

/// The serialised shape of [`InterruptFlags`]: the five booleans the `irq` save-state section has
/// always held, in their original order. Keeping it is what let C7 change the representation
/// without regenerating a single fixture — the same trick as [`crate::timer::TimerSnapshot`].
#[derive(Debug, Clone, Copy, Decode, Encode)]
pub struct InterruptFlagsSnapshot {
    joypad: bool,
    serial: bool,
    timer: bool,
    lcd_stat: bool,
    v_blank: bool,
}

impl InterruptFlags {
    pub const MASK: u8 = 0x1F;

    pub fn snapshot(&self) -> InterruptFlagsSnapshot {
        InterruptFlagsSnapshot {
            joypad: self.is_set(InterruptType::Joypad),
            serial: self.is_set(InterruptType::Serial),
            timer: self.is_set(InterruptType::Timer),
            lcd_stat: self.is_set(InterruptType::LcdStatus),
            v_blank: self.is_set(InterruptType::VBlank),
        }
    }

    pub fn from_snapshot(snapshot: InterruptFlagsSnapshot) -> Self {
        let mut flags = Self::default();
        for (set, interrupt) in [
            (snapshot.joypad, InterruptType::Joypad),
            (snapshot.serial, InterruptType::Serial),
            (snapshot.timer, InterruptType::Timer),
            (snapshot.lcd_stat, InterruptType::LcdStatus),
            (snapshot.v_blank, InterruptType::VBlank),
        ] {
            if set {
                flags.set_interrupt(interrupt);
            }
        }
        flags
    }

    pub fn set(&mut self, value: u8) {
        self.0 = value & Self::MASK;
    }

    pub fn get(&self) -> u8 {
        self.0
    }

    #[inline]
    pub fn is_set(&self, interrupt: InterruptType) -> bool {
        self.0 & interrupt.mask() != 0
    }

    #[inline]
    pub fn clear_interrupt(&mut self, interrupt: InterruptType) {
        self.0 &= !interrupt.mask();
    }

    #[inline]
    pub fn set_interrupt(&mut self, interrupt: InterruptType) {
        self.0 |= interrupt.mask();
    }

    /// The highest-priority interrupt that is both requested (`self`) and enabled (`enabled`).
    ///
    /// One `and` and a `trailing_zeros`, replacing a five-iteration scan that ran once per
    /// instruction. Lowest set bit wins, which *is* the hardware priority order.
    #[inline]
    pub fn highest_priority(&self, enabled: InterruptFlags) -> Option<InterruptType> {
        InterruptType::from_bit(self.0 & enabled.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, strum_macros::EnumIter)]
pub enum InterruptType {
    VBlank,
    LcdStatus,
    Timer,
    Serial,
    Joypad,
}

impl InterruptType {
    pub fn all() -> InterruptTypeIter {
        Self::iter()
    }

    #[inline]
    pub fn mask(self) -> u8 {
        match self {
            InterruptType::VBlank => 0x01,
            InterruptType::LcdStatus => 0x02,
            InterruptType::Timer => 0x04,
            InterruptType::Serial => 0x08,
            InterruptType::Joypad => 0x10,
        }
    }

    /// The lowest set bit of `bits`, as an interrupt. `None` if no bit is set.
    #[inline]
    pub fn from_bit(bits: u8) -> Option<InterruptType> {
        match (bits & InterruptFlags::MASK).trailing_zeros() {
            0 => Some(InterruptType::VBlank),
            1 => Some(InterruptType::LcdStatus),
            2 => Some(InterruptType::Timer),
            3 => Some(InterruptType::Serial),
            4 => Some(InterruptType::Joypad),
            _ => None,
        }
    }

    pub fn address(self) -> u16 {
        match self {
            InterruptType::VBlank => 0x0040,
            InterruptType::LcdStatus => 0x0048,
            InterruptType::Timer => 0x0050,
            InterruptType::Serial => 0x0058,
            InterruptType::Joypad => 0x0060,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interrupt_flags() {
        let mut flags = InterruptFlags::default();
        assert_eq!(flags.get(), 0x00); // No flags set
        flags.set(0x10);
        assert!(flags.is_set(InterruptType::Joypad));
        flags.set(0x08);
        assert!(flags.is_set(InterruptType::Serial));
        flags.set(0x04);
        assert!(flags.is_set(InterruptType::Timer));
        flags.set(0x02);
        assert!(flags.is_set(InterruptType::LcdStatus));
        flags.set(0x01);
        assert!(flags.is_set(InterruptType::VBlank));
        flags.set(0x1F);
        assert_eq!(flags.get(), 0x1F); // All flags set
    }

    /// C7: `highest_priority` replaced a scan in `InterruptType::all()` order, so it has to agree
    /// with that scan on every one of the 1024 (request, enable) combinations — not just the easy
    /// ones. VBlank outranks everything; an interrupt that is requested but not enabled loses.
    #[test]
    fn highest_priority_matches_a_scan_in_priority_order() {
        for request in 0..=0x1Fu8 {
            for enable in 0..=0x1Fu8 {
                let (mut req, mut ena) = (InterruptFlags::default(), InterruptFlags::default());
                req.set(request);
                ena.set(enable);

                let expected = InterruptType::all()
                    .find(|&i| ena.is_set(i) && req.is_set(i));
                assert_eq!(req.highest_priority(ena), expected, "req {request:#04x} ie {enable:#04x}");
            }
        }
    }

    /// Every bit position must round-trip, or an interrupt would be serviced at another's vector.
    #[test]
    fn masks_and_bit_positions_agree() {
        for interrupt in InterruptType::all() {
            assert_eq!(InterruptType::from_bit(interrupt.mask()), Some(interrupt));
        }
        assert_eq!(InterruptType::from_bit(0), None);
        assert_eq!(InterruptType::from_bit(0xE0), None, "IE's unwired top three bits are not interrupts");
    }

    /// The save-state shape is the pre-C7 five booleans; anything else would break 91 fixtures.
    #[test]
    fn a_snapshot_round_trips() {
        for bits in 0..=0x1Fu8 {
            let mut flags = InterruptFlags::default();
            flags.set(bits);
            assert_eq!(InterruptFlags::from_snapshot(flags.snapshot()), flags);
        }
    }
}
